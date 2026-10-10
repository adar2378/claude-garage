//! Terminal startup/shutdown plumbing (task 1.1; port of the stty handling
//! in `tui/lib/bootstrap.dart`, minus the parts crossterm already covers).
//!
//! Ordering: daemon health check (in main, BEFORE touching the terminal) →
//! raw mode + alt screen + bracketed paste → run → full restore on every
//! exit path (normal return, error, panic hook).
//!
//! IXON/ISIG: crossterm's raw mode (cfmakeraw) clears both — verified in the
//! spike via the post-quit `stty -a` check — so Ctrl+Q/Ctrl+S/Ctrl+C reach
//! the application as bytes. IXOFF is NOT in cfmakeraw's clear mask, so it
//! is cleared explicitly here, AFTER `enable_raw_mode` (crossterm snapshots
//! the original termios at enable time, so its `disable_raw_mode` restores
//! the pre-TUI flags — ixoff included — exactly).

use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

/// True while there is nothing to restore (before `enter`, after `restore`).
static RESTORED: AtomicBool = AtomicBool::new(true);

pub type WallTerminal = Terminal<CrosstermBackend<std::io::Stdout>>;

/// Enter the TUI terminal state. Call [`install_panic_hook`] first.
pub fn enter() -> std::io::Result<WallTerminal> {
    enable_raw_mode()?;
    // Mouse capture (SGR) drives task 4.4's click/wheel routing — tile
    // engage/migrate, rail focus, badge → queue, overlay rows, wheel peek.
    crossterm::execute!(
        stdout(),
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    // p14 (spec tui-key-routing "Enhanced-keyboard passthrough"): without
    // the kitty disambiguate flag the outer terminal reports Shift+Enter as
    // plain Enter — Claude Code's newline binding is unreachable. Guarded:
    // a terminal without kitty support just ignores the query, and the
    // probe tells us not to push (pushing unconditionally would leave
    // legacy terminals fine, but the probe keeps event parsing honest).
    if matches!(crossterm::terminal::supports_keyboard_enhancement(), Ok(true)) {
        let _ = crossterm::execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    RESTORED.store(false, Ordering::SeqCst);
    // IXOFF is outside cfmakeraw's mask — clear it ourselves (see module
    // docs; ixon/isig are already off, listed defensively). Runs after
    // enable_raw_mode so crossterm's saved termios still carries the
    // original flags for restore.
    #[cfg(unix)]
    let _ = std::process::Command::new("/bin/sh")
        .args(["-c", "stty -ixon -ixoff -isig < /dev/tty"])
        .status();
    Terminal::new(CrosstermBackend::new(stdout()))
}

/// Full terminal restore. Idempotent — safe to call from the panic hook, the
/// normal exit path, and error paths in any order.
pub fn restore() {
    if RESTORED.swap(true, Ordering::SeqCst) {
        return;
    }
    // Pop is a no-op stack pop on terminals where nothing was pushed —
    // safe to emit unconditionally, and it must run BEFORE leaving the alt
    // screen so the flags never leak into the user's shell.
    let _ = crossterm::execute!(
        stdout(),
        PopKeyboardEnhancementFlags,
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    // Restores the termios snapshot taken by enable_raw_mode: icanon/echo/
    // ixon/ixoff/isig all come back to their pre-TUI values.
    let _ = disable_raw_mode();
}

/// Chain a terminal-restoring panic hook in front of the default one, so a
/// panicking wall never leaves the user's shell in raw mode / on the alt
/// screen (and the panic message is readable). TileClient teardown stays
/// detach-first on panics too: unwinding drops the tiles, whose `Drop`
/// detaches before closing masters.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

/// Open a URL in the user's default browser.
pub fn open_url(url: &str) {
    #[cfg(windows)]
    let _ = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("open").arg(url).spawn();
}

/// Local wall-clock hour (0-23), or `None` when it cannot be determined.
pub fn local_hour() -> Option<u8> {
    #[cfg(windows)]
    {
        let mut t = unsafe { std::mem::zeroed::<windows_sys::Win32::Foundation::SYSTEMTIME>() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
        u8::try_from(t.wHour).ok()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("date")
            .arg("+%H")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u8>().ok())
    }
}

/// Put `text` on the system clipboard. Returns whether it worked.
pub fn copy_to_clipboard(text: &str) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
        };
        use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
        const CF_UNICODETEXT: u32 = 13;
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * 2;
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return false;
            }
            let mut ok = false;
            EmptyClipboard();
            let mem = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if !mem.is_null() {
                let ptr = GlobalLock(mem) as *mut u16;
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                    GlobalUnlock(mem);
                    // On success the system owns `mem`.
                    ok = !SetClipboardData(CF_UNICODETEXT, mem).is_null();
                }
            }
            CloseClipboard();
            ok
        }
    }
    #[cfg(not(windows))]
    {
        use std::io::Write as _;
        let spawned = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn();
        match spawned {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait().map(|s| s.success()).unwrap_or(false)
            }
            Err(_) => false,
        }
    }
}
