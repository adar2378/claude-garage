//! Startup reveal (p23, spec tui-motion "Daemon already running"): a
//! wordmark and a throbber, drawn only while the launcher is actually
//! starting the daemon and the wall is waiting for it to answer.
//!
//! The launcher (`bin/garage.js`) starts the daemon detached, sets
//! `GARAGE_DAEMON_STARTING=1` and hands the terminal straight to the wall;
//! the wall waits for `/api/health` here instead of the launcher doing it
//! blind. The reveal ends the moment the daemon answers — there is no fixed
//! delay, and with the daemon already up (no env var, or healthy on the first
//! probe) none of this runs.

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};
use tachyonfx::{Effect, Interpolation, fx};
use tui_big_text::{BigText, PixelSize};

use crate::api::client::daemon_healthy;
use crate::ui::motion::spinner_span;
use crate::ui::theme::colors;

/// Same budget as the launcher's old `waitForHealth`.
pub const START_DEADLINE: Duration = Duration::from_secs(15);
const POLL_EVERY: Duration = Duration::from_millis(250);
const FRAME: Duration = Duration::from_millis(33);
/// The wordmark settles in this long (a `coalesce`), then holds still.
pub const REVEAL_MS: u32 = 600;
const WORD: &str = "garage";
const TAGLINE: &str = "pit wall for Claude Code sessions";

/// How the wait for the daemon ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Ready,
    Aborted,
    TimedOut,
    /// The daemon process the launcher spawned exited before it answered.
    DaemonDied,
}

/// The pid the launcher spawned (`GARAGE_DAEMON_PID`), if it said.
pub fn daemon_pid_from_env() -> Option<u32> {
    parse_daemon_pid(std::env::var("GARAGE_DAEMON_PID").ok().as_deref())
}

/// The value of `GARAGE_DAEMON_PID` as a usable pid (not 0, not pid 1).
fn parse_daemon_pid(value: Option<&str>) -> Option<u32> {
    value?.trim().parse().ok().filter(|p| *p > 1)
}

/// True when the spawned daemon is certainly dead: `kill(pid, 0)` says no
/// such process. Unix only; elsewhere (or with no pid) never "gone", so the
/// wait falls back to the deadline.
#[cfg(unix)]
pub fn daemon_gone(pid: Option<u32>) -> bool {
    let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()).filter(|p| *p > 1) else {
        return false;
    };
    // SAFETY: signal 0 only checks existence/permission; nothing is sent.
    unsafe {
        libc::kill(pid, 0) != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
}

#[cfg(not(unix))]
pub fn daemon_gone(_pid: Option<u32>) -> bool {
    false
}

/// Whether the reveal runs at all: the launcher said it is starting the
/// daemon, motion is on, and the daemon is not already answering.
pub fn should_reveal(launcher_starting: bool, motion: bool, daemon_up: bool) -> bool {
    launcher_starting && motion && !daemon_up
}

/// The wordmark's pixel size for `width` columns, or `None` when it cannot
/// fit and the plain word is drawn instead.
fn pixel_size_for(width: u16) -> Option<PixelSize> {
    let need = |per_char: u16| per_char * WORD.len() as u16 + 2;
    if width >= need(8) {
        Some(PixelSize::HalfHeight)
    } else if width >= need(4) {
        Some(PixelSize::Quadrant)
    } else {
        None
    }
}

/// Paint the wordmark block (word, tagline, spinner line) centred in `area`.
/// Returns the rect the wordmark itself occupies (where the reveal effect
/// runs).
pub fn render_wordmark(buf: &mut Buffer, area: Rect, now_ms: i64, spinning: bool) -> Rect {
    let size = pixel_size_for(area.width);
    let word_rows: u16 = if size.is_some() { 4 } else { 1 };
    let total = word_rows + 3; // word, gap, tagline, spinner line
    let top = area.y + area.height.saturating_sub(total) / 2;
    let word = Rect::new(area.x, top, area.width, word_rows.min(area.height));
    match size {
        Some(size) => BigText::builder()
            .pixel_size(size)
            .style(Style::default().fg(colors::FG))
            .lines(vec![Line::from(WORD)])
            .centered()
            .build()
            .render(word, buf),
        None => Paragraph::new(Line::styled(
            WORD,
            Style::default().fg(colors::FG).add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center)
        .render(word, buf),
    }
    let mut line = |row: u16, line: Line<'static>| {
        let y = top + row;
        if y < area.bottom() {
            Paragraph::new(line)
                .alignment(Alignment::Center)
                .render(Rect::new(area.x, y, area.width, 1), buf);
        }
    };
    line(
        word_rows + 1,
        Line::styled(TAGLINE, Style::default().fg(colors::FAINT)),
    );
    let waiting = if spinning {
        Line::from(vec![
            spinner_span(now_ms).style(Style::default().fg(colors::DIM)),
            ratatui::text::Span::styled(" starting daemon…", Style::default().fg(colors::DIM)),
        ])
    } else {
        Line::styled("starting daemon…", Style::default().fg(colors::DIM))
    };
    line(word_rows + 2, waiting);
    word
}

fn reveal_effect() -> Effect {
    fx::coalesce((REVEAL_MS, Interpolation::QuadOut))
}

/// Draw the reveal until the daemon answers. Raw mode and the alternate
/// screen are already entered (`term::enter`); on `Ready` they stay so the
/// wall continues on the same terminal.
pub fn reveal(
    terminal: &mut crate::term::WallTerminal,
    base_url: &str,
    deadline: Duration,
    daemon_pid: Option<u32>,
) -> io::Result<Outcome> {
    let started = Instant::now();
    let mut last_poll = Instant::now() - POLL_EVERY;
    let mut last_frame = Instant::now();
    let mut effect = reveal_effect();
    loop {
        if last_poll.elapsed() >= POLL_EVERY {
            last_poll = Instant::now();
            if daemon_healthy(base_url, Duration::from_millis(200)) {
                return Ok(Outcome::Ready);
            }
        }
        if daemon_gone(daemon_pid) {
            return Ok(Outcome::DaemonDied);
        }
        if started.elapsed() >= deadline {
            return Ok(Outcome::TimedOut);
        }
        let step = last_frame.elapsed();
        last_frame = Instant::now();
        let now_ms = started.elapsed().as_millis() as i64;
        terminal.draw(|f| {
            let area = f.area();
            let word = render_wordmark(f.buffer_mut(), area, now_ms, true);
            if !effect.done() {
                effect.process(
                    tachyonfx::Duration::from_millis(step.as_millis().min(100) as _),
                    f.buffer_mut(),
                    word,
                );
            }
        })?;
        if event::poll(FRAME)?
            && let Event::Key(key) = event::read()?
        {
            let ctrl_c =
                key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
            // Only Ctrl+C aborts; every other key is dropped.
            if key.kind != KeyEventKind::Release && ctrl_c {
                return Ok(Outcome::Aborted);
            }
        }
    }
}

/// Wait for the daemon without drawing (motion off): the launcher's old
/// behaviour, on the normal terminal.
pub fn wait_plain(base_url: &str, deadline: Duration, daemon_pid: Option<u32>) -> Outcome {
    let started = Instant::now();
    while started.elapsed() < deadline {
        if daemon_healthy(base_url, Duration::from_millis(200)) {
            return Outcome::Ready;
        }
        if daemon_gone(daemon_pid) {
            return Outcome::DaemonDied;
        }
        std::thread::sleep(POLL_EVERY);
    }
    Outcome::TimedOut
}

/// The message when the daemon never came up (the launcher's wording).
pub fn timed_out_message(base_url: &str, log: Option<&str>) -> String {
    match log {
        Some(log) => format!(
            "claude-garage daemon did not come up at {base_url} — see {log} \
             (port already in use? set GARAGE_PORT to pick a different one)"
        ),
        None => format!("claude-garage daemon did not come up at {base_url}"),
    }
}

/// The message when the spawned daemon exited during startup.
pub fn daemon_died_message(log: Option<&str>) -> String {
    match log {
        Some(log) => format!("claude-garage daemon exited during startup — see {log}"),
        None => "claude-garage daemon exited during startup".to_owned(),
    }
}

#[cfg(test)]
mod tests;
