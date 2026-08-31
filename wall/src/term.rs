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
    RESTORED.store(false, Ordering::SeqCst);
    // IXOFF is outside cfmakeraw's mask — clear it ourselves (see module
    // docs; ixon/isig are already off, listed defensively). Runs after
    // enable_raw_mode so crossterm's saved termios still carries the
    // original flags for restore.
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
    let _ = crossterm::execute!(
        stdout(),
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
