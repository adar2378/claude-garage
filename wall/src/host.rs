//! Native session host — the tmux replacement used on Windows.
//!
//! tmux owns garage's sessions on macOS/Linux. It has no native Windows
//! build, so on Windows each session is run by a small detached `garage-wall
//! host` process instead. A host:
//!
//!  - runs the command (`claude`) on a ConPTY (via `portable-pty`),
//!  - keeps a `vt100` screen + scrollback of everything the command printed,
//!  - listens on a loopback TCP port and registers itself in
//!    `<garage dir>/hosts/<id>.json` so the daemon and the TUI can find it,
//!  - outlives the TUI and the daemon (it is spawned detached), which is what
//!    keeps sessions alive after the terminal window closes.
//!
//! Wire protocol (all integers little-endian). The client opens with a hello:
//! `"GH1"` + 32-char token + mode (`'A'` attach, `'Q'` control) + cols u16 +
//! rows u16. After that, client → host is a stream of frames
//! `[type u8][len u32][payload]`:
//!
//!  - `0` input bytes (attach)
//!  - `1` resize: cols u16, rows u16 (attach)
//!  - `2` history size (control; reply `[len u32][i64]`)
//!  - `3` capture: top i64, cols u16, rows u16 (control; reply `[len u32][text]`)
//!  - `4` shutdown (control; reply `[len u32=1][1]`)
//!  - `5` rename, payload = new id utf8 (control; reply `[len u32=1][1]`)
//!  - `6` type: payload utf8 is pasted into the command's input once its
//!    prompt box is on screen, without pressing Enter (control; reply `[1]`)
//!
//! An attach connection first receives a snapshot of the screen
//! (`state_formatted`), then the raw output stream — snapshot and
//! registration happen under one lock, so no byte is lost or duplicated.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

const MAGIC: &[u8; 3] = b"GH1";
const MODE_ATTACH: u8 = b'A';
const MODE_CONTROL: u8 = b'Q';

const F_INPUT: u8 = 0;
const F_RESIZE: u8 = 1;
const F_HISTORY: u8 = 2;
const F_CAPTURE: u8 = 3;
const F_SHUTDOWN: u8 = 4;
const F_RENAME: u8 = 5;
const F_TYPE: u8 = 6;

const SCROLLBACK: usize = 10_000;

/// True when sessions run on native hosts rather than tmux.
pub fn use_host_backend() -> bool {
    cfg!(windows) || std::env::var("GARAGE_BACKEND").as_deref() == Ok("host")
}

fn io_err(msg: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(msg.to_string())
}

// ── paths / registry ───────────────────────────────────────────────────

/// `$GARAGE_DIR`, else `~/.garage` — same rule as the daemon.
pub fn garage_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("GARAGE_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    home_dir().join(".garage")
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub fn hosts_dir() -> PathBuf {
    garage_dir().join("hosts")
}

/// Session ids contain `/` (`garage/<ws>/<label>`) — not valid in a file name.
pub fn file_stem(id: &str) -> String {
    id.replace('/', "__")
}

/// Where a session's unsent prompt is kept (survives a power cut).
pub fn draft_path(id: &str) -> PathBuf {
    hosts_dir().join(format!("{}.draft", file_stem(id)))
}

/// Clipboard images pasted into a session's prompt (kept with its draft).
pub fn images_dir(id: &str) -> PathBuf {
    hosts_dir().join(format!("{}.images", file_stem(id)))
}

fn registry_path(id: &str) -> PathBuf {
    hosts_dir().join(format!("{}.json", file_stem(id)))
}

#[derive(Clone, Debug)]
pub struct HostInfo {
    pub port: u16,
    pub token: String,
}

/// Look a live host up in the registry.
pub fn read_host(id: &str) -> Option<HostInfo> {
    let text = std::fs::read_to_string(registry_path(id)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(HostInfo {
        port: u16::try_from(v.get("port")?.as_u64()?).ok()?,
        token: v.get("token")?.as_str()?.to_owned(),
    })
}

// ── framing ────────────────────────────────────────────────────────────

pub fn write_frame(w: &mut impl Write, ty: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(5 + payload.len());
    buf.push(ty);
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

fn read_frame(r: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head)?;
    let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(io_err("frame too large"));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok((head[0], payload))
}

fn read_reply(r: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_le_bytes(len) as usize;
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

fn write_reply(w: &mut impl Write, payload: &[u8]) -> std::io::Result<()> {
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

// ── client side ────────────────────────────────────────────────────────

/// Connect to a session's host and complete the hello.
pub fn connect(id: &str, mode: u8, cols: u16, rows: u16) -> std::io::Result<TcpStream> {
    let info = read_host(id).ok_or_else(|| io_err(format!("no session host for {id}")))?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], info.port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_nodelay(true).ok();
    let mut hello = Vec::with_capacity(3 + 32 + 1 + 4);
    hello.extend_from_slice(MAGIC);
    hello.extend_from_slice(info.token.as_bytes());
    hello.push(mode);
    hello.extend_from_slice(&cols.to_le_bytes());
    hello.extend_from_slice(&rows.to_le_bytes());
    stream.write_all(&hello)?;
    Ok(stream)
}

pub fn connect_attach(id: &str, cols: u16, rows: u16) -> std::io::Result<TcpStream> {
    connect(id, MODE_ATTACH, cols, rows)
}

/// Absolute history size (lines scrolled off the top) of a session.
pub fn history_size(id: &str) -> Option<i64> {
    let mut s = connect(id, MODE_CONTROL, 0, 0).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write_frame(&mut s, F_HISTORY, &[]).ok()?;
    let reply = read_reply(&mut s).ok()?;
    Some(i64::from_le_bytes(reply.try_into().ok()?))
}

/// `rows` formatted lines starting at absolute line `top_abs`.
pub fn capture(id: &str, top_abs: i64, cols: u16, rows: u16) -> Option<Vec<u8>> {
    let mut s = connect(id, MODE_CONTROL, 0, 0).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&top_abs.to_le_bytes());
    p.extend_from_slice(&cols.to_le_bytes());
    p.extend_from_slice(&rows.to_le_bytes());
    write_frame(&mut s, F_CAPTURE, &p).ok()?;
    read_reply(&mut s).ok()
}

// ── host side ──────────────────────────────────────────────────────────

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

struct Meta {
    id: String,
    dir: String,
    port: u16,
    token: String,
    pid: u32,
    child_pid: u32,
    created_at: u128,
}

struct HostCtx {
    core: Mutex<Core>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    meta: Mutex<Meta>,
    title: Arc<Mutex<String>>,
    stop: AtomicBool,
}

impl HostCtx {
    fn write_registry(&self) {
        let meta = self.meta.lock().unwrap();
        let title = self.title.lock().unwrap().clone();
        let doc = serde_json::json!({
            "id": meta.id,
            "pid": meta.pid,
            "childPid": meta.child_pid,
            "port": meta.port,
            "token": meta.token,
            "dir": meta.dir,
            "createdAt": meta.created_at,
            "title": title,
        });
        let path = registry_path(&meta.id);
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, doc.to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    fn remove_registry(&self) {
        let meta = self.meta.lock().unwrap();
        let _ = std::fs::remove_file(registry_path(&meta.id));
    }

    fn resize(&self, cols: u16, rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        let _ = self.master.lock().unwrap().resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.core.lock().unwrap().parser.screen_mut().set_size(rows, cols);
    }
}

struct Opts {
    id: String,
    dir: String,
    cols: u16,
    rows: u16,
    command: Vec<String>,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut id = None;
    let mut dir = None;
    let (mut cols, mut rows) = (120u16, 40u16);
    let mut command = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--id" => {
                i += 1;
                id = args.get(i).cloned();
            }
            "--dir" => {
                i += 1;
                dir = args.get(i).cloned();
            }
            "--cols" => {
                i += 1;
                cols = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(cols);
            }
            "--rows" => {
                i += 1;
                rows = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(rows);
            }
            "--" => {
                command = args[i + 1..].to_vec();
                break;
            }
            other => return Err(format!("unknown host option {other}")),
        }
        i += 1;
    }
    match (id, dir) {
        (Some(id), Some(dir)) if !command.is_empty() => Ok(Opts { id, dir, cols, rows, command }),
        _ => Err("usage: host --id <id> --dir <dir> [--cols N --rows N] -- <command...>".into()),
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // Extremely unlikely; fall back to time+pid so the host still starts.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes.copy_from_slice(&(t ^ u128::from(std::process::id())).to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Resolve the program to spawn. On Windows a bare `claude` is usually an npm
/// `.cmd` shim, which `CreateProcess` cannot run directly: look through the
/// shim for the real `.exe` (so the pid we spawn is the pid `claude agents`
/// reports), else fall back to `cmd.exe /c`.
fn resolve_command(command: &[String]) -> Vec<String> {
    #[cfg(windows)]
    {
        if let Some(found) = windows_which(&command[0]) {
            let lower = found.to_string_lossy().to_lowercase();
            if lower.ends_with(".cmd") || lower.ends_with(".bat") {
                if let Some(exe) = exe_behind_shim(&found) {
                    let mut out = vec![exe.to_string_lossy().into_owned()];
                    out.extend_from_slice(&command[1..]);
                    return out;
                }
                let mut out = vec![
                    "cmd.exe".to_owned(),
                    "/d".to_owned(),
                    "/c".to_owned(),
                    found.to_string_lossy().into_owned(),
                ];
                out.extend_from_slice(&command[1..]);
                return out;
            }
            let mut out = vec![found.to_string_lossy().into_owned()];
            out.extend_from_slice(&command[1..]);
            return out;
        }
    }
    command.to_vec()
}

#[cfg(windows)]
fn windows_which(name: &str) -> Option<PathBuf> {
    let p = std::path::Path::new(name);
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
        for e in &exts {
            let mut s = base.clone().into_os_string();
            s.push(e);
            let c = PathBuf::from(s);
            if c.is_file() {
                return Some(c);
            }
        }
        None
    };
    if p.components().count() > 1 {
        return candidates(p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        if let Some(c) = candidates(dir.join(name)) {
            return Some(c);
        }
    }
    None
}

#[cfg(windows)]
fn exe_behind_shim(shim: &std::path::Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(shim).ok()?;
    let dir = shim.parent()?;
    for line in text.lines() {
        let low = line.to_lowercase();
        if !low.contains(".exe\"") {
            continue;
        }
        let start = line.find('"')? + 1;
        let end = start + line[start..].find('"')?;
        let raw = line[start..end].replace("%dp0%", &dir.to_string_lossy());
        let cand = PathBuf::from(raw.replace("\\\\", "\\"));
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Answer the few terminal queries a ConPTY child may block on (the real
/// terminal is not attached while no TUI is watching).
fn answer_queries(ctx: &HostCtx, chunk: &[u8]) {
    let wants_cpr = chunk.windows(4).any(|w| w == b"\x1b[6n");
    let wants_da = chunk.windows(3).any(|w| w == b"\x1b[c")
        || chunk.windows(4).any(|w| w == b"\x1b[0c");
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

fn handle_conn(ctx: Arc<HostCtx>, mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut hello = [0u8; 3 + 32 + 1 + 4];
    stream.read_exact(&mut hello)?;
    if &hello[..3] != MAGIC {
        return Err(io_err("bad magic"));
    }
    let token_ok = {
        let meta = ctx.meta.lock().unwrap();
        hello[3..35] == *meta.token.as_bytes()
    };
    if !token_ok {
        return Err(io_err("bad token"));
    }
    let mode = hello[35];
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
        let mut core = ctx.core.lock().unwrap();
        core.clients.retain(|c| c.peer_addr().map(|a| a != peer).unwrap_or(false));
        return result;
    }

    // control connection
    loop {
        let (ty, payload) = read_frame(&mut stream)?;
        match ty {
            F_HISTORY => {
                let mut core = ctx.core.lock().unwrap();
                let screen = core.parser.screen_mut();
                let cur = screen.scrollback();
                screen.set_scrollback(usize::MAX);
                let n = screen.scrollback() as i64;
                screen.set_scrollback(cur);
                drop(core);
                write_reply(&mut stream, &n.to_le_bytes())?;
            }
            F_CAPTURE if payload.len() >= 12 => {
                let top = i64::from_le_bytes(payload[..8].try_into().unwrap());
                let cols = u16::from_le_bytes([payload[8], payload[9]]);
                let rows = u16::from_le_bytes([payload[10], payload[11]]);
                let text = capture_lines(&ctx, top, cols, rows);
                write_reply(&mut stream, &text)?;
            }
            F_SHUTDOWN => {
                write_reply(&mut stream, &[1])?;
                ctx.stop.store(true, Ordering::SeqCst);
                kill_tree(ctx.meta.lock().unwrap().child_pid);
                return Ok(());
            }
            F_TYPE => {
                type_when_ready(ctx.clone(), String::from_utf8_lossy(&payload).into_owned());
                write_reply(&mut stream, &[1])?;
            }
            F_RENAME => {
                let new_id = String::from_utf8_lossy(&payload).into_owned();
                let old = ctx.meta.lock().unwrap().id.clone();
                let _ = std::fs::remove_file(registry_path(&old));
                ctx.meta.lock().unwrap().id = new_id;
                ctx.write_registry();
                write_reply(&mut stream, &[1])?;
            }
            _ => return Err(io_err("unknown control frame")),
        }
    }
}

fn attach_loop(ctx: &Arc<HostCtx>, stream: &mut TcpStream) -> std::io::Result<()> {
    loop {
        let (ty, payload) = read_frame(stream)?;
        match ty {
            F_INPUT => {
                // Ctrl+V / Alt+V make Claude read an image off the clipboard;
                // keep our own copy so a power cut cannot lose a screenshot.
                if payload.contains(&0x16) || payload.windows(2).any(|w| w == b"\x1bv") {
                    save_clipboard_image(ctx.clone());
                }
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

/// Lines `top_abs..top_abs+rows` in absolute coordinates (0 = oldest retained
/// history line; the live screen's top row sits at `history`), joined by `\n`.
fn capture_lines(ctx: &HostCtx, top_abs: i64, cols: u16, rows: u16) -> Vec<u8> {
    let mut core = ctx.core.lock().unwrap();
    let screen = core.parser.screen_mut();
    let cur = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let hist = screen.scrollback() as i64;
    let offset = (hist - top_abs).max(0) as usize;
    screen.set_scrollback(offset);
    let lines: Vec<Vec<u8>> = screen
        .rows_formatted(0, cols)
        .take(usize::from(rows))
        .collect();
    screen.set_scrollback(cur);
    lines.join(&b'\n')
}

fn is_separator(line: &str) -> bool {
    let t = line.trim();
    t.chars().count() >= 10 && t.chars().all(|c| c == '─')
}

/// The unsent text in Claude Code's prompt box: the rows between the last two
/// full-width `────` rules, minus the `❯ ` prompt. Dim/coloured text (the
/// grey "Try …" suggestion) is not something the user typed and is ignored.
pub fn extract_draft(screen: &vt100::Screen) -> Option<String> {
    let (rows, cols) = screen.size();
    let lines: Vec<String> = screen.rows(0, cols).collect();
    let seps: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_separator(l))
        .map(|(i, _)| i)
        .collect();
    let (a, b) = seps.windows(2).rev().map(|w| (w[0], w[1])).find(|(a, b)| b - a >= 2)?;
    let mut out: Vec<String> = Vec::new();
    for r in (a + 1)..b {
        let row = u16::try_from(r).ok()?;
        if row >= rows {
            break;
        }
        let line = &lines[r];
        let body: &str = if r == a + 1 {
            line.strip_prefix('❯').or_else(|| line.strip_prefix('>'))?.trim_start_matches(' ')
        } else {
            line.strip_prefix("  ").unwrap_or(line)
        };
        if body.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        // Suggestions ("Try ...") are drawn dim; anything dim on the row, or
        // the suggestion's wording, means the user has not typed here.
        if body.trim_start().starts_with("Try \"") {
            return None;
        }
        let any_dim = (0..cols).any(|c| {
            screen
                .cell(row, c)
                .is_some_and(|cell| !cell.contents().trim().is_empty() && cell.dim())
        });
        if any_dim {
            return None;
        }
        out.push(body.trim_end().to_owned());
    }
    let text = out.join("\n").trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// The prompt box's typed text as a selectable region: `(start, end, text)`
/// in screen (row, col) cells — what "select all" highlights and copies.
pub fn draft_selection(screen: &vt100::Screen) -> Option<((u16, u16), (u16, u16), String)> {
    let text = extract_draft(screen)?;
    let (_, cols) = screen.size();
    let lines: Vec<String> = screen.rows(0, cols).collect();
    let seps: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_separator(l))
        .map(|(i, _)| i)
        .collect();
    let (a, b) = seps.windows(2).rev().map(|w| (w[0], w[1])).find(|(a, b)| b - a >= 2)?;
    let last = ((a + 1)..b).rev().find(|&r| !lines[r].trim().is_empty())?;
    let end_col = lines[last].trim_end().chars().count().saturating_sub(1);
    Some((
        (u16::try_from(a + 1).ok()?, 2),
        (u16::try_from(last).ok()?, u16::try_from(end_col).ok()?),
        text,
    ))
}

/// Write (or clear) the session's draft file when it changed.
fn sync_draft(ctx: &HostCtx, last: &mut Option<String>) {
    let now = extract_draft(ctx.core.lock().unwrap().parser.screen());
    if &now == last {
        return;
    }
    let id = ctx.meta.lock().unwrap().id.clone();
    let path = draft_path(&id);
    match &now {
        Some(text) => {
            let tmp = path.with_extension("draft.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        None => {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_dir_all(images_dir(&id));
        }
    }
    *last = now;
}

/// Paste `text` into the command's prompt (no Enter) once its prompt box is
/// on screen — used to put a recovered draft back after a restore.
fn type_when_ready(ctx: Arc<HostCtx>, text: String) {
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while std::time::Instant::now() < deadline {
            let ready = {
                let core = ctx.core.lock().unwrap();
                let (_, cols) = core.parser.screen().size();
                let n = core.parser.screen().rows(0, cols).filter(|l| is_separator(l)).count();
                n >= 2
            };
            if ready {
                std::thread::sleep(Duration::from_millis(600));
                // Parts are separated by NUL: the draft text, then one image
                // path per part (Claude attaches a pasted image path).
                for part in text.split('\0').filter(|p| !p.is_empty()) {
                    {
                        let mut w = ctx.writer.lock().unwrap();
                        let _ = w.write_all(format!("\x1b[200~{part}\x1b[201~").as_bytes());
                        let _ = w.flush();
                    }
                    std::thread::sleep(Duration::from_millis(700));
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}

/// The image on the clipboard as PNG bytes (Windows: CF_DIB), if any.
#[cfg(windows)]
fn clipboard_png() -> Option<Vec<u8>> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    const CF_DIB: u32 = 8;
    unsafe {
        if IsClipboardFormatAvailable(CF_DIB) == 0 || OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let mut result = None;
        let handle = GetClipboardData(CF_DIB);
        if !handle.is_null() {
            let size = GlobalSize(handle);
            let ptr = GlobalLock(handle) as *const u8;
            if !ptr.is_null() && size >= 40 {
                let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
                result = dib_to_png(&bytes);
            }
            GlobalUnlock(handle);
        }
        CloseClipboard();
        result
    }
}

#[cfg(not(windows))]
fn clipboard_png() -> Option<Vec<u8>> {
    None
}

/// Convert a packed DIB (24/32-bit, uncompressed or bitfields) to PNG.
fn dib_to_png(dib: &[u8]) -> Option<Vec<u8>> {
    let u32le = |o: usize| dib.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let header = u32le(0)? as usize;
    let width = i32::from_le_bytes(dib.get(4..8)?.try_into().ok()?);
    let height = i32::from_le_bytes(dib.get(8..12)?.try_into().ok()?);
    let bpp = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?) as usize;
    let compression = u32le(16)?;
    if width <= 0 || height == 0 || !(bpp == 24 || bpp == 32) || !(compression == 0 || compression == 3) {
        return None;
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let bottom_up = height > 0;
    let mut offset = header;
    if compression == 3 && header == 40 {
        offset += 12; // the three channel masks follow a 40-byte header
    }
    let stride = (w * bpp).div_ceil(32) * 4;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let src_row = if bottom_up { h - 1 - y } else { y };
        let row = dib.get(offset + src_row * stride..offset + src_row * stride + w * bpp / 8)?;
        for px in row.chunks_exact(bpp / 8) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().ok()?.write_image_data(&rgba).ok()?;
    Some(out)
}

/// Save the clipboard image next to the session's draft (off-thread: Claude
/// reads the clipboard right after the key, so wait a beat first).
fn save_clipboard_image(ctx: Arc<HostCtx>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        let Some(png) = clipboard_png() else { return };
        let id = ctx.meta.lock().unwrap().id.clone();
        let dir = images_dir(&id);
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        // Same image pasted twice in a row: keep one copy.
        let newest = std::fs::read_dir(&dir)
            .ok()
            .and_then(|d| d.flatten().map(|e| e.path()).max());
        if newest.and_then(|p| std::fs::read(p).ok()).is_some_and(|prev| prev == png) {
            return;
        }
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = std::fs::write(dir.join(format!("img-{ms}.png")), png);
    });
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
    {
        let _ = std::process::Command::new("kill").arg(pid.to_string()).output();
    }
}

/// `garage-wall host ...` — never returns normally; exits with the child's
/// exit code once the session ends.
pub fn run_host(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("garage host: {e}");
            return 2;
        }
    };
    match run_host_inner(opts) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("garage host: {e}");
            1
        }
    }
}

fn run_host_inner(opts: Opts) -> std::io::Result<i32> {
    std::fs::create_dir_all(hosts_dir())?;
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
    let mut child = pair.slave.spawn_command(cmd).map_err(io_err)?;
    let child_pid = child.process_id().unwrap_or(0);
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(io_err)?;
    let writer = pair.master.take_writer().map_err(io_err)?;

    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let title = Arc::new(Mutex::new(String::new()));

    let ctx = Arc::new(HostCtx {
        core: Mutex::new(Core {
            parser: vt100::Parser::new_with_callbacks(
                opts.rows,
                opts.cols,
                SCROLLBACK,
                TitleCb { title: title.clone() },
            ),
            clients: Vec::new(),
        }),
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        meta: Mutex::new(Meta {
            id: opts.id.clone(),
            dir: opts.dir.clone(),
            port,
            token: random_token(),
            pid: std::process::id(),
            child_pid,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        }),
        title,
        stop: AtomicBool::new(false),
    });
    ctx.write_registry();

    // accept loop
    {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let _ = handle_conn(ctx, stream);
                });
            }
        });
    }

    // draft saver: once a second, keep the unsent prompt on disk so a power
    // cut or crash does not lose it (restored by the daemon on restore).
    {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut last: Option<String> = None;
            while !ctx.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(1));
                sync_draft(&ctx, &mut last);
            }
        });
    }

    // output pump: PTY → parser → every attached client
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
    // Let the pump flush the child's last output before clients are cut.
    std::thread::sleep(Duration::from_millis(200));
    // A session that ended normally has nothing left to recover.
    ctx.stop.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_file(draft_path(&ctx.meta.lock().unwrap().id));
    ctx.remove_registry();
    for c in ctx.core.lock().unwrap().clients.drain(..) {
        let _ = c.shutdown(Shutdown::Both);
    }
    Ok(code)
}

#[cfg(test)]
mod draft_tests {
    use super::*;

    fn screen_with(prompt_rows: &str) -> vt100::Parser {
        let mut p = vt100::Parser::new(10, 40, 0);
        let rule = "─".repeat(30);
        p.process(format!("some output\r\n{rule}\r\n{prompt_rows}\r\n{rule}\r\nfooter").as_bytes());
        p
    }

    #[test]
    fn typed_text_is_extracted_and_selectable() {
        let p = screen_with("❯ hello world");
        assert_eq!(extract_draft(p.screen()).as_deref(), Some("hello world"));
        let (start, end, text) = draft_selection(p.screen()).unwrap();
        assert_eq!(text, "hello world");
        assert_eq!(start, (2, 2));
        assert_eq!(end, (2, 12));
    }

    #[test]
    fn multi_line_drafts_keep_their_newlines() {
        let p = screen_with("❯ first line\r\n  second line");
        assert_eq!(extract_draft(p.screen()).as_deref(), Some("first line\nsecond line"));
        let (_, end, _) = draft_selection(p.screen()).unwrap();
        assert_eq!(end.0, 3);
    }

    #[test]
    fn dim_suggestion_is_not_a_draft() {
        let p = screen_with("❯ \x1b[2mTry \"fix a bug\"\x1b[0m");
        assert_eq!(extract_draft(p.screen()), None);
    }

    #[test]
    fn empty_prompt_is_not_a_draft() {
        let p = screen_with("❯ ");
        assert_eq!(extract_draft(p.screen()), None);
        assert!(draft_selection(p.screen()).is_none());
    }
}
