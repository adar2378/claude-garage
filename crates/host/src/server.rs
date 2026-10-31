//! The host process: `claude-garage host --id <id> --dir <dir> -- <command...>`.
//!
//! It runs the command on a PTY (ConPTY on Windows), mirrors everything the
//! command prints into a `vt100` screen with 10k lines of scrollback, serves
//! attach and control connections on a loopback port (see [`crate::proto`]),
//! and keeps `hosts/<id>.json` current. It is spawned detached by the daemon,
//! so it outlives the daemon, the TUI and the terminal window; it exits when
//! the command exits or on a shutdown frame.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::proto::*;
use crate::registry::{self, Entry};

pub const SCROLLBACK: usize = 10_000;

/// Whole budget for a connection to send its complete hello (not per read).
const HELLO_DEADLINE: Duration = Duration::from_secs(5);
/// Connections allowed to sit unauthenticated at once; more are closed at once.
const MAX_PENDING: usize = 8;
/// Largest terminal size a client may ask for.
pub const MAX_COLS: u16 = 1000;
pub const MAX_ROWS: u16 = 500;

/// A size a client asked for, or `None` if it is zero in either dimension.
/// Oversized values are clamped: a hostile client must not make the vt100
/// screen allocate `65535 x 65535` cells.
pub fn sane_size(cols: u16, rows: u16) -> Option<(u16, u16)> {
    (cols > 0 && rows > 0).then(|| (cols.min(MAX_COLS), rows.min(MAX_ROWS)))
}

/// Constant-time equality for equal-length secrets (no early exit on a
/// mismatching byte); different lengths are simply unequal.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Counts connections that have not authenticated yet.
#[derive(Default)]
struct Pending(Arc<AtomicUsize>);

/// One unauthenticated-connection slot; freed on drop (or earlier, once the
/// connection has authenticated).
struct Slot(Option<Arc<AtomicUsize>>);

impl Pending {
    fn try_acquire(&self, max: usize) -> Option<Slot> {
        if self.0.fetch_add(1, Ordering::SeqCst) >= max {
            self.0.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Slot(Some(self.0.clone())))
    }
}

impl Slot {
    fn release(&mut self) {
        if let Some(n) = self.0.take() {
            n.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.release();
    }
}

/// Read exactly `buf.len()` bytes before `deadline`, however the bytes are
/// spread over reads: a client trickling one byte at a time gets no extra time.
fn read_before(stream: &mut TcpStream, buf: &mut [u8], deadline: Instant) -> std::io::Result<()> {
    let mut got = 0;
    while got < buf.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "hello deadline",
            ));
        }
        stream.set_read_timeout(Some(left))?;
        match stream.read(&mut buf[got..]) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Parsed `host` arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub id: String,
    pub dir: String,
    pub hosts_dir: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub command: Vec<String>,
}

/// `--id <id> --dir <dir> --hosts-dir <dir> [--cols N --rows N] -- <command...>`
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let (mut id, mut dir, mut hosts_dir) = (None, None, None);
    let (mut cols, mut rows) = (120u16, 40u16);
    let mut command = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[i]))
        };
        match args[i].as_str() {
            "--id" => id = Some(value(i)?),
            "--dir" => dir = Some(value(i)?),
            "--hosts-dir" => hosts_dir = Some(PathBuf::from(value(i)?)),
            "--cols" => {
                cols = value(i)?
                    .parse()
                    .map_err(|_| "--cols must be a number".to_string())?
            }
            "--rows" => {
                rows = value(i)?
                    .parse()
                    .map_err(|_| "--rows must be a number".to_string())?
            }
            "--" => {
                command = args[i + 1..].to_vec();
                break;
            }
            other => return Err(format!("unknown host option {other}")),
        }
        i += 2;
    }
    match (id, dir, hosts_dir) {
        (Some(id), Some(dir), Some(hosts_dir)) if !command.is_empty() && sane_size(cols, rows) == Some((cols, rows)) => {
            Ok(Options { id, dir, hosts_dir, cols, rows, command })
        }
        _ => Err("usage: host --id <id> --dir <dir> --hosts-dir <dir> [--cols N --rows N] -- <command...>".into()),
    }
}

/// `claude-garage host ...`: returns the process exit code.
pub fn run_cli(args: &[String]) -> i32 {
    let opts = match parse_args(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("garage host: {e}");
            return 2;
        }
    };
    match run(opts) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("garage host: {e}");
            1
        }
    }
}

// ---- screen helpers (pure over a parser) ----

/// Lines scrolled off the top.
pub fn history_len<CB: vt100::Callbacks>(parser: &mut vt100::Parser<CB>) -> i64 {
    let screen = parser.screen_mut();
    let cur = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let n = screen.scrollback() as i64;
    screen.set_scrollback(cur);
    n
}

/// Lines `top_abs..top_abs+rows` in absolute coordinates (0 = oldest retained
/// history line; the live screen's top row sits at `history`), with escapes,
/// joined by `\n`.
pub fn formatted_window<CB: vt100::Callbacks>(
    parser: &mut vt100::Parser<CB>,
    top_abs: i64,
    cols: u16,
    rows: u16,
) -> Vec<u8> {
    let hist = history_len(parser);
    let screen = parser.screen_mut();
    let cur = screen.scrollback();
    screen.set_scrollback((hist - top_abs).max(0) as usize);
    let lines: Vec<Vec<u8>> = screen
        .rows_formatted(0, cols)
        .take(usize::from(rows))
        .collect();
    screen.set_scrollback(cur);
    lines.join(&b'\n')
}

/// The last `n` lines of scrollback plus screen as plain text, without
/// trailing blank lines (the shape `tmux capture-pane -p` gives).
pub fn text_tail<CB: vt100::Callbacks>(parser: &mut vt100::Parser<CB>, n: usize) -> String {
    let hist = history_len(parser) as usize;
    let screen = parser.screen_mut();
    let (rows, cols) = screen.size();
    let rows = usize::from(rows).max(1);
    let total = hist + rows;
    let cur = screen.scrollback();
    let mut all: Vec<String> = Vec::with_capacity(total);
    let mut abs = 0;
    while abs < total {
        // A window of `rows` lines starting at `abs` (or at `hist`, the live
        // screen, once `abs` has passed the history).
        let offset = hist.saturating_sub(abs);
        screen.set_scrollback(offset);
        let top = hist - offset;
        let skip = abs - top;
        let take = (rows - skip).min(total - abs);
        all.extend(screen.rows(0, cols).skip(skip).take(take));
        abs += take;
    }
    screen.set_scrollback(cur);
    while all.last().is_some_and(|l| l.trim().is_empty()) {
        all.pop();
    }
    let start = all.len().saturating_sub(n);
    all[start..].join("\n")
}

// ---- the host ----

struct TitleCb {
    title: Arc<Mutex<String>>,
}

impl vt100::Callbacks for TitleCb {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        *self.title.lock().unwrap() = String::from_utf8_lossy(title).into_owned();
    }
}

struct Core {
    parser: vt100::Parser<TitleCb>,
    clients: Vec<TcpStream>,
}

struct Ctx {
    core: Mutex<Core>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    meta: Mutex<Entry>,
    hosts_dir: PathBuf,
    title: Arc<Mutex<String>>,
    stop: AtomicBool,
    /// Set once the command has exited, so its pid (possibly reused by then)
    /// is never signalled.
    exited: AtomicBool,
    pending: Pending,
}

impl Ctx {
    fn write_registry(&self) {
        let mut meta = self.meta.lock().unwrap();
        meta.title = self.title.lock().unwrap().clone();
        let _ = registry::write(&self.hosts_dir, &meta);
    }

    fn resize(&self, cols: u16, rows: u16) {
        let Some((cols, rows)) = sane_size(cols, rows) else {
            return;
        };
        let _ = self.master.lock().unwrap().resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.core
            .lock()
            .unwrap()
            .parser
            .screen_mut()
            .set_size(rows, cols);
    }

    fn kill_command(&self) {
        if self.exited.load(Ordering::SeqCst) {
            return;
        }
        let child_pid = self.meta.lock().unwrap().child_pid;
        kill_tree(child_pid);
        let _ = self.killer.lock().unwrap().kill();
    }

    fn info(&self) -> Info {
        let meta = self.meta.lock().unwrap().clone();
        let attached = self.core.lock().unwrap().clients.len() as u32;
        Info {
            id: meta.id,
            pid: meta.pid,
            child_pid: meta.child_pid,
            dir: meta.dir,
            created_at: meta.created_at,
            title: self.title.lock().unwrap().clone(),
            attached,
        }
    }
}

/// The session token. The OS random source failing is fatal: a guessable
/// token would be an open door, so the host refuses to start instead.
fn random_token() -> std::io::Result<String> {
    let mut bytes = [0u8; TOKEN_LEN / 2];
    getrandom::fill(&mut bytes)
        .map_err(|e| io_err(format!("no secure randomness for the session token: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Candidate `.exe` paths named in an npm `.cmd` shim (`"%dp0%\...\claude.exe"`),
/// in file order. Pure so it can be tested on any OS.
pub fn exe_candidates_in_shim(text: &str, shim_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in text.lines() {
        if !line.to_lowercase().contains(".exe\"") {
            continue;
        }
        let Some(start) = line.find('"').map(|i| i + 1) else {
            continue;
        };
        let Some(len) = line[start..].find('"') else {
            continue;
        };
        let raw = line[start..start + len].replace("%dp0%", &shim_dir.to_string_lossy());
        out.push(PathBuf::from(raw.replace("\\\\", "\\")));
    }
    out
}

/// Resolve the program to spawn. On Windows a bare `claude` is usually an npm
/// `.cmd` shim, which `CreateProcess` cannot run directly: look through the
/// shim for the real `.exe` (so the pid we own is the pid `claude agents`
/// reports), else fall back to `cmd.exe /c`. Elsewhere the command is used as is.
pub fn resolve_command(command: &[String]) -> Vec<String> {
    #[cfg(windows)]
    {
        if let Some(found) = windows_which(&command[0]) {
            let lower = found.to_string_lossy().to_lowercase();
            let mut out = if lower.ends_with(".cmd") || lower.ends_with(".bat") {
                let exe = std::fs::read_to_string(&found).ok().and_then(|text| {
                    exe_candidates_in_shim(&text, found.parent()?)
                        .into_iter()
                        .find(|c| c.is_file())
                });
                match exe {
                    Some(exe) => vec![exe.to_string_lossy().into_owned()],
                    None => vec![
                        "cmd.exe".into(),
                        "/d".into(),
                        "/c".into(),
                        found.to_string_lossy().into_owned(),
                    ],
                }
            } else {
                vec![found.to_string_lossy().into_owned()]
            };
            out.extend_from_slice(&command[1..]);
            return out;
        }
    }
    command.to_vec()
}

#[cfg(windows)]
fn windows_which(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    let has_ext = p.extension().is_some();
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .map(|s| s.to_lowercase())
        .collect();
    let candidates = |base: PathBuf| -> Option<PathBuf> {
        if has_ext && base.is_file() {
            return Some(base);
        }
        exts.iter().find_map(|e| {
            let mut s = base.clone().into_os_string();
            s.push(e);
            let c = PathBuf::from(s);
            c.is_file().then_some(c)
        })
    };
    if p.components().count() > 1 {
        return candidates(p.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| candidates(dir.join(name)))
}

/// Answer the few terminal queries a ConPTY child may block on (the real
/// terminal is not attached while no TUI is watching).
fn answer_queries(ctx: &Ctx, chunk: &[u8]) {
    let wants_cpr = chunk.windows(4).any(|w| w == b"\x1b[6n");
    let wants_da =
        chunk.windows(3).any(|w| w == b"\x1b[c") || chunk.windows(4).any(|w| w == b"\x1b[0c");
    if !wants_cpr && !wants_da {
        return;
    }
    let mut reply = Vec::new();
    if wants_cpr {
        let (r, c) = ctx.core.lock().unwrap().parser.screen().cursor_position();
        reply.extend_from_slice(format!("\x1b[{};{}R", r + 1, c + 1).as_bytes());
    }
    if wants_da {
        reply.extend_from_slice(b"\x1b[?1;2c");
    }
    let mut w = ctx.writer.lock().unwrap();
    let _ = w.write_all(&reply);
    let _ = w.flush();
}

fn handle_conn(ctx: Arc<Ctx>, mut stream: TcpStream, mut slot: Slot) -> std::io::Result<()> {
    let mut hello = [0u8; 3 + TOKEN_LEN + 1 + 4];
    read_before(&mut stream, &mut hello, Instant::now() + HELLO_DEADLINE)?;
    if &hello[..3] != MAGIC {
        return Err(io_err("bad magic"));
    }
    if !ct_eq(
        &hello[3..3 + TOKEN_LEN],
        ctx.meta.lock().unwrap().token.as_bytes(),
    ) {
        return Err(io_err("bad token"));
    }
    slot.release();
    let mode = hello[3 + TOKEN_LEN];
    let cols = u16::from_le_bytes([hello[36], hello[37]]);
    let rows = u16::from_le_bytes([hello[38], hello[39]]);
    stream.set_read_timeout(None)?;
    stream.set_nodelay(true).ok();

    if mode == MODE_ATTACH {
        ctx.resize(cols, rows);
        let peer = stream.peer_addr()?;
        {
            let mut core = ctx.core.lock().unwrap();
            let mut out = stream.try_clone()?;
            out.set_write_timeout(Some(Duration::from_secs(2)))?;
            out.write_all(&core.parser.screen().state_formatted())?;
            core.clients.push(out);
        }
        let result = attach_loop(&ctx, &mut stream);
        ctx.core
            .lock()
            .unwrap()
            .clients
            .retain(|c| c.peer_addr().is_ok_and(|a| a != peer));
        return result;
    }

    // control connection: any number of request/reply round trips
    loop {
        let (ty, payload) = read_frame(&mut stream)?;
        match ty {
            F_HISTORY => {
                let n = history_len(&mut ctx.core.lock().unwrap().parser);
                write_reply(&mut stream, &n.to_le_bytes())?;
            }
            F_ANCHOR => {
                let mut core = ctx.core.lock().unwrap();
                let history = history_len(&mut core.parser);
                let cursor = i64::from(core.parser.screen().cursor_position().0);
                let mut reply = Vec::with_capacity(24);
                for n in [history, cursor, SCROLLBACK as i64] {
                    reply.extend_from_slice(&n.to_le_bytes());
                }
                drop(core);
                write_reply(&mut stream, &reply)?;
            }
            F_CAPTURE if payload.len() >= 12 => {
                let top = i64::from_le_bytes(payload[..8].try_into().unwrap());
                let cols = u16::from_le_bytes([payload[8], payload[9]]);
                let rows = u16::from_le_bytes([payload[10], payload[11]]);
                let text = formatted_window(&mut ctx.core.lock().unwrap().parser, top, cols, rows);
                write_reply(&mut stream, &text)?;
            }
            F_TEXT if payload.len() >= 4 => {
                let n = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
                let text = text_tail(&mut ctx.core.lock().unwrap().parser, n);
                write_reply(&mut stream, text.as_bytes())?;
            }
            F_INFO => {
                let json = serde_json::to_vec(&ctx.info()).map_err(io_err)?;
                write_reply(&mut stream, &json)?;
            }
            F_SHUTDOWN => {
                write_reply(&mut stream, &[1])?;
                ctx.stop.store(true, Ordering::SeqCst);
                ctx.kill_command();
                return Ok(());
            }
            F_RENAME => {
                let new_id = String::from_utf8_lossy(&payload).into_owned();
                let old = std::mem::replace(&mut ctx.meta.lock().unwrap().id, new_id);
                let mut gone = ctx.meta.lock().unwrap().clone();
                gone.id = old;
                let _ = registry::remove_if_same(&ctx.hosts_dir, &gone);
                ctx.write_registry();
                write_reply(&mut stream, &[1])?;
            }
            _ => return Err(io_err("unknown control frame")),
        }
    }
}

fn attach_loop(ctx: &Arc<Ctx>, stream: &mut TcpStream) -> std::io::Result<()> {
    loop {
        let (ty, payload) = read_frame(stream)?;
        match ty {
            F_INPUT => {
                let mut w = ctx.writer.lock().unwrap();
                w.write_all(&payload)?;
                w.flush()?;
            }
            F_RESIZE if payload.len() >= 4 => {
                ctx.resize(
                    u16::from_le_bytes([payload[0], payload[1]]),
                    u16::from_le_bytes([payload[2], payload[3]]),
                );
            }
            _ => {}
        }
    }
}

fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: the host has no console, so taskkill would flash one.
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .output();
    }
    #[cfg(not(windows))]
    let _ = pid; // portable-pty's killer signals the child on Unix
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Run one host to completion; returns the command's exit code. Blocks until
/// the command exits. Safe to call on a thread (tests do).
pub fn run(opts: Options) -> std::io::Result<i32> {
    registry::ensure_dir(&opts.hosts_dir)?;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: opts.rows,
            cols: opts.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(io_err)?;

    let argv = resolve_command(&opts.command);
    let mut cmd = CommandBuilder::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.cwd(&opts.dir);
    cmd.env("COLORTERM", "truecolor");
    #[cfg(not(windows))]
    cmd.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(cmd).map_err(io_err)?;
    let child_pid = child.process_id().unwrap_or(0);
    let killer = child.clone_killer();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(io_err)?;
    let writer = pair.master.take_writer().map_err(io_err)?;

    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let title = Arc::new(Mutex::new(String::new()));
    let token = random_token()?;

    let ctx = Arc::new(Ctx {
        core: Mutex::new(Core {
            parser: vt100::Parser::new_with_callbacks(
                opts.rows,
                opts.cols,
                SCROLLBACK,
                TitleCb {
                    title: title.clone(),
                },
            ),
            clients: Vec::new(),
        }),
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        killer: Mutex::new(killer),
        meta: Mutex::new(Entry {
            id: opts.id.clone(),
            pid: std::process::id(),
            child_pid,
            port,
            token,
            dir: opts.dir.clone(),
            created_at: now_ms(),
            title: String::new(),
        }),
        hosts_dir: opts.hosts_dir.clone(),
        title,
        stop: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        pending: Pending::default(),
    });
    ctx.write_registry();

    // accept loop: polls so it can stop (and free the port) when the host ends
    {
        let ctx = ctx.clone();
        listener.set_nonblocking(true)?;
        std::thread::spawn(move || {
            while !ctx.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        // Too many unauthenticated connections: drop this one now.
                        let Some(slot) = ctx.pending.try_acquire(MAX_PENDING) else {
                            continue;
                        };
                        let ctx = ctx.clone();
                        std::thread::spawn(move || {
                            let _ = handle_conn(ctx, stream, slot);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // output pump: PTY -> parser -> every attached client
    {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            let mut last_title = String::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        {
                            let mut core = ctx.core.lock().unwrap();
                            core.parser.process(&buf[..n]);
                            core.clients.retain_mut(|c| c.write_all(&buf[..n]).is_ok());
                        }
                        answer_queries(&ctx, &buf[..n]);
                        let t = ctx.title.lock().unwrap().clone();
                        if t != last_title {
                            last_title = t;
                            ctx.write_registry();
                        }
                    }
                }
            }
        });
    }

    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.exit_code() as i32,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break 1,
        }
    };
    ctx.exited.store(true, Ordering::SeqCst);
    // Let the pump flush the command's last output before clients are cut.
    std::thread::sleep(Duration::from_millis(200));
    ctx.stop.store(true, Ordering::SeqCst);
    // Our own record only: after a respawn the same id may belong to a new host.
    let own = ctx.meta.lock().unwrap().clone();
    let _ = registry::remove_if_same(&ctx.hosts_dir, &own);
    for c in ctx.core.lock().unwrap().clients.drain(..) {
        let _ = c.shutdown(Shutdown::Both);
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn args_parse_with_defaults_and_everything_after_the_separator_is_the_command() {
        let o = parse_args(&s(&[
            "--id",
            "garage/w/a",
            "--dir",
            "/w",
            "--hosts-dir",
            "/h",
            "--",
            "claude",
            "--resume",
            "x",
        ]))
        .unwrap();
        assert_eq!(o.id, "garage/w/a");
        assert_eq!((o.cols, o.rows), (120, 40));
        assert_eq!(o.hosts_dir, PathBuf::from("/h"));
        assert_eq!(o.command, ["claude", "--resume", "x"]);
        let o = parse_args(&s(&[
            "--id",
            "i",
            "--dir",
            "d",
            "--hosts-dir",
            "h",
            "--cols",
            "80",
            "--rows",
            "24",
            "--",
            "x",
        ]))
        .unwrap();
        assert_eq!((o.cols, o.rows), (80, 24));
    }

    #[test]
    fn bad_args_are_usage_errors() {
        assert!(
            parse_args(&s(&["--id", "i", "--dir", "d", "--hosts-dir", "h"])).is_err(),
            "no command"
        );
        assert!(
            parse_args(&s(&["--dir", "d", "--hosts-dir", "h", "--", "x"])).is_err(),
            "no id"
        );
        assert!(
            parse_args(&s(&[
                "--id",
                "i",
                "--dir",
                "d",
                "--hosts-dir",
                "h",
                "--cols",
                "0",
                "--",
                "x"
            ]))
            .is_err()
        );
        assert!(
            parse_args(&s(&[
                "--id",
                "i",
                "--dir",
                "d",
                "--hosts-dir",
                "h",
                "--cols",
                "wide",
                "--",
                "x"
            ]))
            .is_err()
        );
        assert!(parse_args(&s(&["--bogus", "1", "--", "x"])).is_err());
        assert!(parse_args(&s(&["--id"])).is_err(), "flag without a value");
    }

    #[test]
    fn shim_parsing_finds_the_exe_and_expands_dp0() {
        let shim = "@ECHO off\r\nSETLOCAL\r\n\"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\"   %*\r\n";
        let got = exe_candidates_in_shim(shim, Path::new("C:\\npm"));
        assert_eq!(
            got,
            [PathBuf::from(
                "C:\\npm\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe"
            )]
        );
        assert!(exe_candidates_in_shim("@echo off\nnode x.js", Path::new("C:\\npm")).is_empty());
    }

    #[test]
    fn sizes_are_clamped_and_zero_is_ignored() {
        assert_eq!(sane_size(80, 24), Some((80, 24)));
        assert_eq!(sane_size(65535, 65535), Some((MAX_COLS, MAX_ROWS)));
        assert_eq!(sane_size(0, 24), None);
        assert_eq!(sane_size(80, 0), None);
        assert!(
            parse_args(&s(&[
                "--id",
                "i",
                "--dir",
                "d",
                "--hosts-dir",
                "h",
                "--cols",
                "5000",
                "--",
                "x"
            ]))
            .is_err()
        );
    }

    #[test]
    fn token_compare_is_exact() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(!ct_eq(b"", b"a"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn unauthenticated_slots_are_capped_and_freed() {
        let p = Pending::default();
        let mut held: Vec<Slot> = (0..3)
            .map(|_| p.try_acquire(3).expect("under the cap"))
            .collect();
        assert!(p.try_acquire(3).is_none(), "the 4th is refused");
        assert_eq!(
            p.0.load(Ordering::SeqCst),
            3,
            "a refusal does not leak a count"
        );
        held[0].release();
        held[0].release(); // idempotent
        assert_eq!(p.0.load(Ordering::SeqCst), 2);
        let again = p.try_acquire(3);
        assert!(again.is_some());
        drop(again);
        held.clear();
        assert_eq!(p.0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_hello_trickled_one_byte_at_a_time_hits_the_whole_deadline() {
        use std::net::TcpListener;
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = l.local_addr().unwrap();
        let writer = std::thread::spawn(move || {
            let mut c = TcpStream::connect(addr).unwrap();
            // Each byte arrives well inside a per-read timeout, never completing the hello.
            for _ in 0..40 {
                if c.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let (mut srv, _) = l.accept().unwrap();
        let started = Instant::now();
        let mut buf = [0u8; 40];
        let err = read_before(
            &mut srv,
            &mut buf,
            Instant::now() + Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(
            matches!(
                err.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ),
            "{err:?}"
        );
        assert!(
            started.elapsed() < Duration::from_millis(900),
            "gave up at the deadline, not per read"
        );
        drop(srv);
        writer.join().unwrap();
    }

    #[test]
    fn a_complete_hello_is_read_even_when_split() {
        use std::net::TcpListener;
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = l.local_addr().unwrap();
        let w = std::thread::spawn(move || {
            let mut c = TcpStream::connect(addr).unwrap();
            c.write_all(b"ab").unwrap();
            std::thread::sleep(Duration::from_millis(50));
            c.write_all(b"cd").unwrap();
        });
        let (mut srv, _) = l.accept().unwrap();
        let mut buf = [0u8; 4];
        read_before(&mut srv, &mut buf, Instant::now() + Duration::from_secs(2)).unwrap();
        assert_eq!(&buf, b"abcd");
        w.join().unwrap();
    }

    fn parser(rows: u16, cols: u16, sb: usize) -> vt100::Parser {
        vt100::Parser::new(rows, cols, sb)
    }

    #[test]
    fn text_tail_returns_history_and_screen_without_trailing_blanks() {
        let mut p = parser(4, 20, 100);
        for i in 0..10 {
            p.process(format!("line {i}\r\n").as_bytes());
        }
        // 10 lines were printed on a 4-row screen: 7 in history (+ blank cursor row).
        assert_eq!(
            text_tail(&mut p, 100),
            (0..10)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(text_tail(&mut p, 3), "line 7\nline 8\nline 9");
        assert_eq!(text_tail(&mut p, 0), "");
        assert!(history_len(&mut p) > 0);
        // Reading must not leave the parser scrolled back.
        assert_eq!(p.screen().scrollback(), 0);
    }

    #[test]
    fn text_tail_of_a_fresh_screen_is_empty() {
        assert_eq!(text_tail(&mut parser(4, 20, 100), 50), "");
    }

    #[test]
    fn formatted_window_addresses_absolute_history_lines() {
        let mut p = parser(4, 20, 100);
        for i in 0..10 {
            p.process(format!("line {i}\r\n").as_bytes());
        }
        let hist = history_len(&mut p);
        let top = formatted_window(&mut p, 0, 20, 2);
        let text = String::from_utf8_lossy(&top).into_owned();
        assert!(
            text.contains("line 0") && text.contains("line 1") && !text.contains("line 2"),
            "{text:?}"
        );
        // The window at `hist` is the live screen's top row.
        let live = String::from_utf8_lossy(&formatted_window(&mut p, hist, 20, 1)).into_owned();
        assert!(live.contains(&format!("line {hist}")), "{live:?}");
    }
}
