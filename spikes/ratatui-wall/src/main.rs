// ratatui-wall — feasibility spike for the claude-garage TUI port.
//
// 3x3 grid of live terminals; each tile is a PTY running `tmux attach -t
// =rspike-N` (TMUX stripped, TERM=xterm-256color). Keys: 1-9 focus, Enter
// engage, Ctrl+G disengage (raw byte passthrough while engaged),
// Shift+PageUp/PageDown frozen scrollback with an ABSOLUTE anchor, q quit.
//
// Scratch sessions only (rspike-*). Never touches the daemon, ~/.garage, or
// the user's sessions. Env: GARAGE_TUI_KEYLOG=<file> appends
// "<epoch_us> <layer> <desc>" per handled key (latency instrumentation,
// same contract as the Dart TUI's keylog).

use std::io::Write as _;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use tui_term::widget::{Cursor, PseudoTerminal};

const N_TILES: usize = 9;
const SCROLLBACK: usize = 5000;
const FRAME_CAP: Duration = Duration::from_millis(33); // ~30fps render cap

enum AppEvent {
    Term(Event),
    Dirty,
}

struct Tile {
    name: String,
    parser: Arc<Mutex<vt100::Parser>>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,
    /// Last size pushed to the PTY (cols, rows) so we only TIOCSWINSZ on change.
    pty_size: (u16, u16),
    /// Frozen local scrollback view (None = live). tmux attach clients live on
    /// the ALT SCREEN and repaint in place, so the vt100 scrollback never
    /// fills — history must be seeded from `tmux capture-pane` (the p8
    /// design's capture-pane seeding), anchored ABSOLUTELY in tmux history
    /// coordinates so live output never drags the view.
    frozen: Option<FrozenView>,
}

struct FrozenView {
    /// Absolute index (into history + visible lines) of the view's top line.
    /// Live output grows history but can never drag this view.
    top_abs: i64,
    /// Pages above the freeze point. PageDown decrements; 0 → back to live
    /// (on a streaming tile the live edge runs away from an absolute anchor,
    /// so "as many pages down as up" is the return path, not chasing it).
    pages_up: u32,
    /// Rendered snapshot of the frozen range (fresh parser, no scrollback).
    parser: vt100::Parser,
}

/// Current tmux history size of an inner session (absolute line 0 = oldest
/// retained history line; the visible top row sits at index history_size).
fn tmux_history_size(session: &str) -> Option<i64> {
    let out = std::process::Command::new("tmux")
        .args(["display-message", "-p", "-t", &format!("={session}:"), "-F", "#{history_size}"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Capture `rows` lines starting at absolute line `top_abs` from the inner
/// session, with escape sequences (-e), rendered into a fresh vt100 parser.
fn capture_frozen(session: &str, top_abs: i64, cols: u16, rows: u16) -> Option<vt100::Parser> {
    let hist = tmux_history_size(session)?;
    let s_rel = top_abs - hist; // ≤ 0 reaches into history
    let e_rel = s_rel + i64::from(rows) - 1;
    let out = std::process::Command::new("tmux")
        .args([
            "capture-pane", "-p", "-e", "-t", &format!("={session}:"),
            "-S", &s_rel.to_string(), "-E", &e_rel.to_string(),
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut parser = vt100::Parser::new(rows, cols, 0);
    let mut first = true;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if !first {
            parser.process(b"\r\n");
        }
        first = false;
        parser.process(line.as_bytes());
        parser.process(b"\x1b[0m"); // don't bleed styles across lines
    }
    Some(parser)
}

enum Layer {
    Garage,
    Engaged(usize),
}

struct KeyLog(Option<std::fs::File>);
impl KeyLog {
    fn new() -> Self {
        KeyLog(std::env::var("GARAGE_TUI_KEYLOG").ok().and_then(|p| {
            std::fs::OpenOptions::new().create(true).append(true).open(p).ok()
        }))
    }
    fn log(&mut self, layer: &str, desc: &str) {
        if let Some(f) = self.0.as_mut() {
            let us = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_micros())
                .unwrap_or(0);
            let _ = writeln!(f, "{} {} {}", us, layer, desc);
            let _ = f.flush();
        }
    }
}

fn main() -> std::io::Result<()> {
    // Spawn all tile PTYs BEFORE entering the alt screen so a spawn failure
    // prints a normal error.
    let (tx, rx) = mpsc::channel::<AppEvent>();
    let mut tiles = Vec::with_capacity(N_TILES);
    for i in 0..N_TILES {
        tiles.push(spawn_tile(i + 1, tx.clone())?);
    }

    // Input thread: crossterm event stream -> channel.
    {
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            match event::read() {
                Ok(ev) => {
                    if tx.send(AppEvent::Term(ev)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        });
    }

    let mut terminal = ratatui::init();
    // ratatui::init gives raw mode (termios via crossterm: ISIG/IXON/ICANON/
    // ECHO all cleared, restored on ratatui::restore) + alt screen.
    crossterm::execute!(std::io::stdout(), event::EnableBracketedPaste).ok();
    let res = run(&mut terminal, &mut tiles, &rx);
    crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste).ok();
    ratatui::restore();

    // Teardown: detach our attach CLIENTS gracefully FIRST, then close the
    // masters. Hard teardown (SIGHUP via portable-pty's kill, or plain
    // master-close EOF) makes the dying client inject an EOF-ish byte into
    // the attached pane (observed: pane received \n + ^D per teardown) — an
    // idle interactive shell reads ^D as EOF, exits 0, and takes its tmux
    // session with it. detach-client sidesteps the injection entirely.
    // (Sessions themselves are never ours to kill.)
    for t in tiles.iter() {
        let _ = std::process::Command::new("tmux")
            .args(["detach-client", "-s", &format!("={}", t.name)])
            .status();
    }
    std::thread::sleep(Duration::from_millis(150));
    drop(tiles.drain(..));
    res
}

fn spawn_tile(n: usize, tx: mpsc::Sender<AppEvent>) -> std::io::Result<Tile> {
    let name = format!("rspike-{n}");
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .map_err(std::io::Error::other)?;
    let mut cmd = CommandBuilder::new("tmux");
    cmd.args(["attach", "-t", &format!("={name}")]);
    cmd.env_remove("TMUX"); // nested-client refusal (nocterm lesson)
    cmd.env("TERM", "xterm-256color");
    let _child = pair.slave.spawn_command(cmd).map_err(std::io::Error::other)?;
    drop(pair.slave);

    let parser = Arc::new(Mutex::new(vt100::Parser::new(24, 80, SCROLLBACK)));
    let mut reader = pair.master.try_clone_reader().map_err(std::io::Error::other)?;
    let writer = pair.master.take_writer().map_err(std::io::Error::other)?;
    {
        let parser = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        parser.lock().unwrap().process(&buf[..n]);
                        if tx.send(AppEvent::Dirty).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
    Ok(Tile {
        name,
        parser,
        master: pair.master,
        writer,
        pty_size: (80, 24),
        frozen: None,
    })
}

/// Grid geometry: full frame minus a 1-line bottom strip; 3 rows x 3 cols.
fn tile_rects(area: Rect) -> (Vec<Rect>, Rect) {
    let [grid, strip] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let rows = Layout::vertical([Constraint::Fill(1); 3]).split(grid);
    let mut rects = Vec::with_capacity(9);
    for r in rows.iter() {
        let cols = Layout::horizontal([Constraint::Fill(1); 3]).split(*r);
        rects.extend(cols.iter().copied());
    }
    (rects, strip)
}

/// Resize each tile PTY (TIOCSWINSZ via portable-pty) + parser to its tile's
/// inner size. Done on startup and every Resize event — the resize bug class.
fn apply_sizes(tiles: &mut [Tile], rects: &[Rect]) {
    for (t, r) in tiles.iter_mut().zip(rects) {
        let inner = Rect {
            x: r.x + 1,
            y: r.y + 1,
            width: r.width.saturating_sub(2),
            height: r.height.saturating_sub(2),
        };
        let (cols, rows) = (inner.width.max(2), inner.height.max(2));
        if t.pty_size != (cols, rows) {
            t.pty_size = (cols, rows);
            let _ = t.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
            t.parser.lock().unwrap().screen_mut().set_size(rows, cols);
        }
    }
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    tiles: &mut Vec<Tile>,
    rx: &mpsc::Receiver<AppEvent>,
) -> std::io::Result<()> {
    let mut keylog = KeyLog::new();
    let mut layer = Layer::Garage;
    let mut focused: usize = 0;
    let mut dirty = true;
    let mut last_render = Instant::now() - FRAME_CAP;

    let size = terminal.size()?;
    let (rects, _) = tile_rects(Rect::new(0, 0, size.width, size.height));
    apply_sizes(tiles, &rects);

    loop {
        // Render (capped) when dirty.
        if dirty && last_render.elapsed() >= FRAME_CAP {
            draw(terminal, tiles, &layer, focused)?;
            last_render = Instant::now();
            dirty = false;
        }
        let timeout = if dirty {
            FRAME_CAP.saturating_sub(last_render.elapsed())
        } else {
            Duration::from_millis(250)
        };
        let first = match rx.recv_timeout(timeout) {
            Ok(ev) => Some(ev),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        // Drain everything pending; handle inputs in order.
        let mut events: Vec<AppEvent> = Vec::new();
        if let Some(e) = first {
            events.push(e);
        }
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        for ev in events {
            match ev {
                AppEvent::Dirty => dirty = true,
                AppEvent::Term(Event::Resize(w, h)) => {
                    let (rects, _) = tile_rects(Rect::new(0, 0, w, h));
                    apply_sizes(tiles, &rects);
                    dirty = true;
                }
                AppEvent::Term(Event::Paste(text)) => {
                    if let Layer::Engaged(i) = layer {
                        let t = &mut tiles[i];
                        let _ = t.writer.write_all(b"\x1b[200~");
                        let _ = t.writer.write_all(text.as_bytes());
                        let _ = t.writer.write_all(b"\x1b[201~");
                        let _ = t.writer.flush();
                    }
                }
                AppEvent::Term(Event::Key(key)) => {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    dirty = true;
                    match layer {
                        Layer::Garage => {
                            keylog.log("garage", &format!("{:?} {:?}", key.code, key.modifiers));
                            match key.code {
                                KeyCode::Char(c @ '1'..='9') => {
                                    focused = c as usize - '1' as usize;
                                }
                                KeyCode::Enter => layer = Layer::Engaged(focused),
                                KeyCode::Char('q') => return Ok(()),
                                KeyCode::Char('c')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    return Ok(());
                                }
                                _ => {}
                            }
                        }
                        Layer::Engaged(i) => {
                            keylog.log("engaged", &format!("{:?} {:?}", key.code, key.modifiers));
                            handle_engaged(&mut tiles[i], &key, &mut layer);
                        }
                    }
                }
                AppEvent::Term(_) => {}
            }
        }
    }
}

fn handle_engaged(tile: &mut Tile, key: &KeyEvent, layer: &mut Layer) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    // Ctrl+G: disengage (never forwarded). No framework intercept to fight —
    // the nocterm Ctrl+G debug-toggle collision does not exist here.
    if ctrl && matches!(key.code, KeyCode::Char('g')) {
        *layer = Layer::Garage;
        return;
    }
    // Shift+PageUp/PageDown: LOCAL frozen scrollback (never forwarded, tmux
    // copy-mode never triggered). The frozen view is a capture-pane snapshot
    // anchored at an ABSOLUTE tmux history index; live output cannot drag it.
    let (cols, rows) = tile.pty_size;
    let page = i64::from(rows.saturating_sub(1)).max(1);
    if shift && matches!(key.code, KeyCode::PageUp) {
        let (top_abs, pages_up) = match &tile.frozen {
            None => ((tmux_history_size(&tile.name).unwrap_or(0) - page).max(0), 1),
            Some(f) => ((f.top_abs - page).max(0), f.pages_up + 1),
        };
        if let Some(parser) = capture_frozen(&tile.name, top_abs, cols, rows) {
            tile.frozen = Some(FrozenView { top_abs, pages_up, parser });
        }
        return;
    }
    if shift && matches!(key.code, KeyCode::PageDown) {
        if let Some(f) = &tile.frozen {
            if f.pages_up <= 1 {
                tile.frozen = None; // back to live
            } else {
                let (top_abs, pages_up) = (f.top_abs + page, f.pages_up - 1);
                if let Some(parser) = capture_frozen(&tile.name, top_abs, cols, rows) {
                    tile.frozen = Some(FrozenView { top_abs, pages_up, parser });
                }
            }
        }
        return;
    }

    // Anything else: typing snaps to live, then verbatim passthrough.
    tile.frozen = None;
    let bytes = encode_key(key);
    if !bytes.is_empty() {
        let _ = tile.writer.write_all(&bytes);
        let _ = tile.writer.flush();
    }
}

/// Re-encode a parsed crossterm key event into the raw bytes a real terminal
/// would send (the verbatim-passthrough contract: modifiers must survive).
fn encode_key(key: &KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let modn: u8 = 1 + (shift as u8) + (alt as u8) * 2 + (ctrl as u8) * 4;
    let mut out = Vec::with_capacity(8);

    let csi = |out: &mut Vec<u8>, fin: u8| {
        if modn == 1 {
            out.extend_from_slice(b"\x1b[");
        } else {
            out.extend_from_slice(format!("\x1b[1;{modn}").as_bytes());
        }
        out.push(fin);
    };
    let tilde = |out: &mut Vec<u8>, num: u8| {
        if modn == 1 {
            out.extend_from_slice(format!("\x1b[{num}~").as_bytes());
        } else {
            out.extend_from_slice(format!("\x1b[{num};{modn}~").as_bytes());
        }
    };

    match key.code {
        KeyCode::Char(c) => {
            if alt {
                out.push(0x1b);
            }
            if ctrl {
                match c.to_ascii_lowercase() {
                    l @ 'a'..='z' => out.push(l as u8 - b'a' + 1),
                    ' ' | '@' => out.push(0x00),
                    '[' => out.push(0x1b),
                    '\\' => out.push(0x1c),
                    ']' => out.push(0x1d),
                    '^' => out.push(0x1e),
                    '_' | '/' => out.push(0x1f),
                    '?' => out.push(0x7f),
                    other => {
                        let mut b = [0u8; 4];
                        out.extend_from_slice(other.encode_utf8(&mut b).as_bytes());
                    }
                }
            } else {
                let mut b = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        }
        KeyCode::Enter => {
            if alt {
                out.push(0x1b);
            }
            out.push(b'\r');
        }
        KeyCode::Tab => {
            if alt {
                out.push(0x1b);
            }
            out.push(b'\t');
        }
        KeyCode::BackTab => out.extend_from_slice(b"\x1b[Z"),
        KeyCode::Backspace => {
            if alt {
                out.push(0x1b);
            }
            out.push(if ctrl { 0x08 } else { 0x7f });
        }
        KeyCode::Esc => out.push(0x1b),
        KeyCode::Up => csi(&mut out, b'A'),
        KeyCode::Down => csi(&mut out, b'B'),
        KeyCode::Right => csi(&mut out, b'C'),
        KeyCode::Left => csi(&mut out, b'D'),
        KeyCode::Home => csi(&mut out, b'H'),
        KeyCode::End => csi(&mut out, b'F'),
        KeyCode::Insert => tilde(&mut out, 2),
        KeyCode::Delete => tilde(&mut out, 3),
        KeyCode::PageUp => tilde(&mut out, 5),
        KeyCode::PageDown => tilde(&mut out, 6),
        KeyCode::F(n @ 1..=4) => {
            // F1-F4: SS3 P/Q/R/S unmodified, CSI 1;mod P.. with modifiers.
            let fin = b'P' + (n as u8 - 1);
            if modn == 1 {
                out.extend_from_slice(b"\x1bO");
                out.push(fin);
            } else {
                out.extend_from_slice(format!("\x1b[1;{modn}").as_bytes());
                out.push(fin);
            }
        }
        KeyCode::F(n @ 5..=12) => {
            let num = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                _ => 24,
            };
            tilde(&mut out, num);
        }
        _ => {}
    }
    out
}

fn draw(
    terminal: &mut ratatui::DefaultTerminal,
    tiles: &mut [Tile],
    layer: &Layer,
    focused: usize,
) -> std::io::Result<()> {
    terminal.draw(|f| {
        let (rects, strip) = tile_rects(f.area());
        for (i, (t, r)) in tiles.iter_mut().zip(rects.iter()).enumerate() {
            let engaged = matches!(layer, Layer::Engaged(e) if *e == i);
            let frozen = t.frozen.is_some();
            let mut title = format!(" {}:{} ", i + 1, t.name);
            if engaged {
                title.push_str(if frozen { "[engaged·frozen] " } else { "[engaged] " });
            }
            let border_style = if engaged {
                Style::default().fg(Color::Green)
            } else if i == focused {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(title);
            // Cursor bug class: only the engaged live tile shows a cursor.
            let cursor = Cursor::default().visibility(engaged && !frozen);
            if let Some(fv) = &t.frozen {
                let term = PseudoTerminal::new(fv.parser.screen()).block(block).cursor(cursor);
                term.render(*r, f.buffer_mut());
            } else {
                let p = t.parser.lock().unwrap();
                let term = PseudoTerminal::new(p.screen()).block(block).cursor(cursor);
                term.render(*r, f.buffer_mut());
            }
        }
        let chip = match layer {
            Layer::Garage => "keys → garage".to_string(),
            Layer::Engaged(i) => format!("keys → {}", tiles[*i].name),
        };
        let hints = match layer {
            Layer::Garage => "1-9 focus · Enter engage · q quit",
            Layer::Engaged(_) => "Ctrl+G disengage · Shift+PgUp/PgDn scrollback",
        };
        let strip_line = Line::from(format!(" {chip}  |  {hints}"));
        Paragraph::new(strip_line)
            .style(Style::default().add_modifier(Modifier::REVERSED))
            .render(strip, f.buffer_mut());
    })?;
    Ok(())
}
