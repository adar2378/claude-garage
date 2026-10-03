//! The worktree-finish overlay (p17, spec tui-worktree-finish): after `x x`
//! closes a session that was spawned into a git worktree, the DELETE
//! response's worktree record lands here and a small modal offers `m`
//! merge, `d d` discard (armed — a second `d` confirms) and `k`/Esc keep.
//! The overlay is modal: the session and its metadata are already gone, so
//! this record is the only handle on the worktree — nothing else on the
//! wall may take the keys until the user resolves it. A daemon refusal
//! (dirty worktree, merge conflict) renders inline and keeps the overlay
//! open so the user can retry, discard or keep.
//!
//! Pure state + content + geometry, like `view_picker`/`workspace_add`:
//! runtime.rs routes the keys, runs the finish effect and applies its
//! settle. Records queue up behind the one on screen, so two quick worktree
//! closes never drop a record.

use std::collections::VecDeque;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::api::models::{FinishAction, WorktreeRecord};
use crate::state::armed_action::ArmedAction;
use crate::ui::layout::{centered_rect, modal_width};
use crate::ui::theme::colors;

/// The single key the discard arm is keyed by (one target at a time — the
/// record on screen).
const DISCARD_ARM: &str = "discard";

/// Daemon errors can be multi-line git stderr (a conflict lists every
/// file); past this many wrapped rows the modal would crowd the wall.
const MAX_ERROR_ROWS: usize = 6;

/// What a key press asks of the runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinishChoice {
    /// Consumed, nothing to do (busy, an unbound key, the first `d`).
    Nothing,
    /// Send `POST /api/worktrees/finish` for this record.
    Run(WorktreeRecord, FinishAction),
    /// The record on screen was kept (no request); `true` when the overlay
    /// has no more records and should close.
    Kept { close: bool },
}

/// The overlay's state: the queue of records (front = on screen), the
/// in-flight action, the inline error and the armed discard.
#[derive(Default)]
pub struct WorktreeFinishState {
    queue: VecDeque<WorktreeRecord>,
    /// `Some` while a finish request is in flight — every choice is ignored.
    pub busy: Option<FinishAction>,
    pub error: Option<String>,
    armed_discard: ArmedAction,
}

impl WorktreeFinishState {
    /// Queue a record behind any already showing. The overlay opens on the
    /// first one; later ones wait their turn.
    pub fn push(&mut self, record: WorktreeRecord) {
        self.queue.push_back(record);
    }

    /// The record on screen.
    pub fn current(&self) -> Option<&WorktreeRecord> {
        self.queue.front()
    }

    /// Records waiting behind the one on screen.
    pub fn waiting(&self) -> usize {
        self.queue.len().saturating_sub(1)
    }

    /// Whether the discard arm is live at `now_ms` (the render's prompt).
    pub fn discard_armed(&self, now_ms: i64) -> bool {
        self.armed_discard.armed_live(now_ms).is_some()
    }

    /// `m`: merge the record on screen (ignored while busy).
    pub fn merge(&mut self) -> FinishChoice {
        self.armed_discard.disarm();
        self.run(FinishAction::Merge)
    }

    /// `d`: the first press arms, a second within the window confirms.
    pub fn discard(&mut self, now_ms: i64) -> FinishChoice {
        if self.busy.is_some() || self.current().is_none() {
            return FinishChoice::Nothing;
        }
        if self.armed_discard.press(DISCARD_ARM, now_ms) {
            return self.run(FinishAction::Discard);
        }
        FinishChoice::Nothing
    }

    /// `k`/Esc: drop the record on screen without any daemon call — the
    /// worktree and branch stay exactly where they are.
    pub fn keep(&mut self) -> FinishChoice {
        if self.busy.is_some() {
            return FinishChoice::Nothing;
        }
        self.advance();
        FinishChoice::Kept { close: self.queue.is_empty() }
    }

    /// Any other key disarms a pending discard (the `x x` rule).
    pub fn other_key(&mut self) {
        self.armed_discard.disarm();
    }

    /// The finish request settled. Success moves on to the next record and
    /// returns `true` when none is left (close the overlay); a failure
    /// keeps the record on screen with the daemon's message inline.
    pub fn settled(&mut self, error: Option<String>) -> bool {
        self.busy = None;
        match error {
            Some(error) => {
                self.error = Some(error);
                false
            }
            None => {
                self.advance();
                self.queue.is_empty()
            }
        }
    }

    fn run(&mut self, action: FinishAction) -> FinishChoice {
        if self.busy.is_some() {
            return FinishChoice::Nothing;
        }
        let Some(record) = self.current().cloned() else {
            return FinishChoice::Nothing;
        };
        self.busy = Some(action);
        self.error = None;
        FinishChoice::Run(record, action)
    }

    fn advance(&mut self) {
        self.queue.pop_front();
        self.error = None;
        self.armed_discard.disarm();
    }
}

/// The strip notice for a successful finish (spec: names the branch and the
/// outcome).
pub fn finish_notice(record: &WorktreeRecord, action: FinishAction) -> String {
    match action {
        FinishAction::Merge => format!("merged {} into {}", record.branch, target_name(record)),
        FinishAction::Discard => format!("discarded {}", record.branch),
    }
}

/// The merge target as shown: the daemon-reported branch, or a plain
/// description when it could not read one.
fn target_name(record: &WorktreeRecord) -> &str {
    record.target.as_deref().unwrap_or("current branch")
}

/// Keep the tail of `s` within `width` chars (paths are most telling at
/// the end).
fn tail(s: &str, width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= width || width == 0 {
        return s.to_owned();
    }
    let keep = width.saturating_sub(1);
    format!("…{}", chars[chars.len() - keep..].iter().collect::<String>())
}

/// Word-wrap every line of `text` to `width` chars, capped at `max` rows.
/// A word longer than `width` (a long path) is hard-split.
fn wrap_rows(text: &str, width: usize, max: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for line in text.lines() {
        let mut row = String::new();
        let mut row_len = 0;
        for word in line.split_whitespace() {
            let chars: Vec<char> = word.chars().collect();
            if row_len > 0 && row_len + 1 + chars.len() > width {
                rows.push(std::mem::take(&mut row));
                row_len = 0;
            }
            for chunk in chars.chunks(width) {
                if row_len > 0 && row_len + 1 + chunk.len() > width {
                    rows.push(std::mem::take(&mut row));
                    row_len = 0;
                }
                if row_len > 0 {
                    row.push(' ');
                    row_len += 1;
                }
                row.extend(chunk);
                row_len += chunk.len();
            }
        }
        if row_len > 0 {
            rows.push(row);
        }
    }
    if rows.len() > max {
        rows.truncate(max);
        if let Some(last) = rows.last_mut() {
            last.push('…');
        }
    }
    rows
}

fn choice_line(key: &'static str, what: String, enabled: bool) -> Line<'static> {
    let what_color = if enabled { colors::DIM } else { colors::FAINT };
    Line::from(vec![
        Span::styled(format!("{key:<4}"), Style::default().fg(colors::FG)),
        Span::styled(what, Style::default().fg(what_color)),
    ])
}

/// Content lines: branch → target, path, the three choices, then the
/// status rows (armed prompt / busy / error / queue) when any apply.
pub fn worktree_finish_lines(
    state: &WorktreeFinishState,
    now_ms: i64,
    width: usize,
) -> Vec<Line<'static>> {
    let Some(record) = state.current() else {
        return Vec::new();
    };
    let target = target_name(record).to_owned();
    let idle = state.busy.is_none();
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                record.branch.clone(),
                Style::default().fg(colors::FG).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" → ", Style::default().fg(colors::DIM)),
            Span::styled(target.clone(), Style::default().fg(colors::FG)),
        ]),
        Line::styled(tail(&record.path, width), Style::default().fg(colors::FAINT)),
        Line::raw(""),
        choice_line("m", format!("merge into {target}"), idle),
        choice_line("d d", "discard branch and worktree".to_owned(), idle),
        choice_line("k", "keep both for later (or Esc)".to_owned(), idle),
    ];

    let mut status: Vec<Line<'static>> = Vec::new();
    if let Some(action) = state.busy {
        let text = match action {
            FinishAction::Merge => format!("merging {}…", record.branch),
            FinishAction::Discard => format!("discarding {}…", record.branch),
        };
        status.push(Line::styled(text, Style::default().fg(colors::FAINT)));
    } else if state.discard_armed(now_ms) {
        // Red, not amber — amber is reserved for needs-input.
        status.push(Line::styled(
            format!("press d again to discard {}", record.branch),
            Style::default().fg(colors::ERROR).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(error) = &state.error {
        for row in wrap_rows(error, width, MAX_ERROR_ROWS) {
            status.push(Line::styled(row, Style::default().fg(colors::ERROR)));
        }
    }
    if state.waiting() > 0 {
        let n = state.waiting();
        status.push(Line::styled(
            format!("{n} more worktree{} after this", if n == 1 { "" } else { "s" }),
            Style::default().fg(colors::FAINT),
        ));
    }
    if !status.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(status);
    }
    lines
}

/// The modal width for an `area` (same rule as the other overlays).
fn finish_modal_width(area: Rect) -> u16 {
    modal_width(area.width, 36, 64)
}

/// The modal rect for the current state — border + padding around the
/// content rows, like `view_picker`.
pub fn worktree_finish_modal_rect(area: Rect, state: &WorktreeFinishState, now_ms: i64) -> Rect {
    let width = finish_modal_width(area);
    let content_width = usize::from(width.saturating_sub(6));
    let rows = worktree_finish_lines(state, now_ms, content_width).len() as u16;
    centered_rect(area, width, (rows + 4).min(area.height))
}

/// Render the overlay; returns the modal rect (clicks anywhere are
/// swallowed — the overlay is modal and has no click targets).
pub fn render_worktree_finish(
    buf: &mut Buffer,
    area: Rect,
    state: &WorktreeFinishState,
    now_ms: i64,
) -> Rect {
    let rect = worktree_finish_modal_rect(area, state, now_ms);
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::DIM))
        .title(Line::styled(" finish worktree ", Style::default().fg(colors::FG)));
    let inner = block.inner(rect);
    block.render(rect, buf);
    let padded = Rect {
        x: inner.x + 2.min(inner.width),
        y: inner.y + 1.min(inner.height),
        width: inner.width.saturating_sub(4),
        height: inner.height.saturating_sub(2),
    };
    Paragraph::new(worktree_finish_lines(state, now_ms, usize::from(padded.width)))
        .render(padded, buf);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(branch: &str, target: Option<&str>) -> WorktreeRecord {
        WorktreeRecord {
            path: format!("/Users/me/.garage/worktrees/kowboy/{}", branch.trim_start_matches("garage/")),
            branch: branch.to_owned(),
            repo_dir: "/Users/me/dev/kowboy".to_owned(),
            target: target.map(str::to_owned),
        }
    }

    fn open(branch: &str) -> WorktreeFinishState {
        let mut state = WorktreeFinishState::default();
        state.push(record(branch, Some("main")));
        state
    }

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn merge_runs_once_and_goes_busy() {
        let mut state = open("garage/feature");
        assert_eq!(
            state.merge(),
            FinishChoice::Run(record("garage/feature", Some("main")), FinishAction::Merge)
        );
        assert_eq!(state.busy, Some(FinishAction::Merge));
        assert_eq!(state.merge(), FinishChoice::Nothing, "busy ignores choices");
        assert_eq!(state.discard(1000), FinishChoice::Nothing);
        assert_eq!(state.keep(), FinishChoice::Nothing);
    }

    #[test]
    fn discard_needs_a_second_d_within_the_window() {
        let mut state = open("garage/feature");
        assert_eq!(state.discard(1000), FinishChoice::Nothing);
        assert!(state.discard_armed(1001));
        assert_eq!(
            state.discard(2000),
            FinishChoice::Run(record("garage/feature", Some("main")), FinishAction::Discard)
        );
    }

    #[test]
    fn another_key_or_an_expired_window_disarms_discard() {
        let mut state = open("garage/feature");
        state.discard(1000);
        state.other_key();
        assert!(!state.discard_armed(1001));
        assert_eq!(state.discard(1100), FinishChoice::Nothing, "re-arms");
        assert!(!state.discard_armed(5000), "expired arm stops prompting");
        assert_eq!(state.discard(5000), FinishChoice::Nothing, "expired: re-arms");
        // Merge also disarms.
        state.merge();
        state.settled(Some("boom".to_owned()));
        assert!(!state.discard_armed(5001));
    }

    #[test]
    fn keep_drops_the_record_and_closes_when_none_wait() {
        let mut state = open("garage/a");
        state.push(record("garage/b", None));
        assert_eq!(state.keep(), FinishChoice::Kept { close: false });
        assert_eq!(state.current().unwrap().branch, "garage/b");
        assert_eq!(state.keep(), FinishChoice::Kept { close: true });
        assert_eq!(state.current(), None);
    }

    #[test]
    fn an_error_keeps_the_record_and_shows_inline() {
        let mut state = open("garage/feature");
        state.merge();
        assert!(!state.settled(Some("worktree has uncommitted changes".to_owned())));
        assert_eq!(state.busy, None);
        assert_eq!(state.current().unwrap().branch, "garage/feature");
        let lines = text(&worktree_finish_lines(&state, 0, 50));
        assert!(lines.contains(&"worktree has uncommitted changes".to_owned()));
        assert!(lines.iter().any(|l| l.starts_with("m   merge into main")), "still offered");
        // A retry clears the error.
        state.merge();
        assert_eq!(state.error, None);
    }

    #[test]
    fn success_advances_and_reports_close() {
        let mut state = open("garage/a");
        state.push(record("garage/b", Some("main")));
        state.merge();
        assert!(!state.settled(None), "one more waiting");
        assert_eq!(state.current().unwrap().branch, "garage/b");
        state.merge();
        assert!(state.settled(None));
    }

    #[test]
    fn notices_name_branch_and_outcome() {
        assert_eq!(
            finish_notice(&record("garage/x", Some("main")), FinishAction::Merge),
            "merged garage/x into main"
        );
        assert_eq!(
            finish_notice(&record("garage/x", None), FinishAction::Merge),
            "merged garage/x into current branch"
        );
        assert_eq!(
            finish_notice(&record("garage/x", Some("main")), FinishAction::Discard),
            "discarded garage/x"
        );
    }

    #[test]
    fn lines_name_branch_target_and_choices() {
        let state = open("garage/feature");
        let lines = text(&worktree_finish_lines(&state, 0, 58));
        assert_eq!(lines[0], "garage/feature → main");
        assert_eq!(lines[3], "m   merge into main");
        assert_eq!(lines[4], "d d discard branch and worktree");
        assert_eq!(lines[5], "k   keep both for later (or Esc)");
        assert_eq!(lines.len(), 6, "no status rows when idle");

        let mut unknown = WorktreeFinishState::default();
        unknown.push(record("garage/feature", None));
        assert_eq!(text(&worktree_finish_lines(&unknown, 0, 58))[0], "garage/feature → current branch");
    }

    #[test]
    fn long_errors_wrap_and_cap() {
        let rows = wrap_rows("abcdef\n\nghij", 4, 6);
        assert_eq!(rows, ["abcd", "ef", "ghij"]);
        let many = "x\n".repeat(10);
        let rows = wrap_rows(&many, 4, 3);
        assert_eq!(rows, ["x", "x", "x…"]);
    }

    #[test]
    fn long_paths_keep_their_tail() {
        assert_eq!(tail("/a/b/c/d", 5), "…/c/d");
        assert_eq!(tail("/a", 5), "/a");
    }

    /// Plain-text dump of a rendered buffer (one row per line, trailing
    /// spaces trimmed) — the overlay's look without a terminal.
    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area;
        let mut out = String::new();
        for y in area.y..area.y + area.height {
            let mut row = String::new();
            for x in area.x..area.x + area.width {
                row.push_str(buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
            }
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }

    /// Render every overlay state into an 80×20 buffer and write the text to
    /// `target/worktree_finish_overlay.txt` (also printed — visible with
    /// `cargo test worktree_finish_snapshot -- --nocapture`), so the look
    /// can be reviewed without driving a live wall.
    #[test]
    fn worktree_finish_snapshot() {
        let area = Rect::new(0, 0, 80, 20);
        let mut states: Vec<(&str, WorktreeFinishState, i64)> = Vec::new();

        states.push(("idle", open("garage/feature"), 0));

        let mut armed = open("garage/feature");
        armed.discard(1000);
        states.push(("discard armed", armed, 1500));

        let mut busy = open("garage/feature");
        busy.merge();
        states.push(("merging", busy, 0));

        let mut error = open("garage/feature");
        error.merge();
        error.settled(Some(
            "merge conflict — merge aborted, nothing changed; resolve it in the session \
             (merge the target into the branch), then try again"
                .to_owned(),
        ));
        error.push(record("garage/other", None));
        states.push(("merge conflict + queued", error, 0));

        let mut no_target = WorktreeFinishState::default();
        no_target.push(record("garage/feature", None));
        states.push(("target unknown", no_target, 0));

        let mut dump = String::new();
        for (name, state, now_ms) in &states {
            let mut buf = Buffer::empty(area);
            let rect = render_worktree_finish(&mut buf, area, state, *now_ms);
            assert!(rect.width <= area.width && rect.height <= area.height);
            let rendered = buffer_text(&buf);
            assert!(rendered.contains("finish worktree"), "{name}: title drawn");
            assert!(rendered.contains("garage/"), "{name}: branch drawn");
            dump.push_str(&format!("── {name} ──\n{rendered}\n"));
        }
        println!("{dump}");
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/target/worktree_finish_overlay.txt");
        let _ = std::fs::write(path, &dump);
    }
}
