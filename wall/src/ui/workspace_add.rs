//! The `w` add-workspace overlay (port of
//! `tui/lib/ui/workspace_add_overlay.dart` + the submit logic from
//! `bin/garage_tui.dart` — spec tui-key-routing "p8.1"): a centered modal
//! with one text field for a directory path. Enter submits (`~` expansion,
//! client-side dir-exists validation, basename name derivation, then the
//! PUT effect); Esc cancels; validation/daemon errors render inline and keep
//! the overlay open. Pastes land in the field cleaned of shell quoting
//! (Finder drag-and-drop into Ghostty arrives backslash-escaped), and Ctrl+O
//! opens the daemon's native folder picker (`POST /api/pick-directory`,
//! macOS-only).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::state::workspace_form::{derive_workspace_name, expand_tilde};
use crate::ui::layout::{centered_rect, modal_width};
use crate::ui::theme::colors;

/// How a Ctrl+O folder-picker call settled — the runtime maps the daemon
/// response onto this so [`WorkspaceAddForm::pick_settled`] stays pure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickOutcome {
    /// `200 {dir}` — the chosen folder's absolute path.
    Picked(String),
    /// `200 {cancelled: true}` — the user dismissed the dialog.
    Cancelled,
    /// `501` — no native picker on this host (non-macOS daemon).
    Unsupported,
    /// Anything else (500, transport error, malformed reply).
    Failed,
}

/// The overlay's state: one text buffer plus the inline error and the
/// in-flight guards. Pure (IO is injected) so the whole submit flow
/// unit-tests without a filesystem or daemon.
#[derive(Default)]
pub struct WorkspaceAddForm {
    pub input: String,
    pub error: Option<String>,
    pub busy: bool,
    /// A Ctrl+O folder-picker call is in flight (the osascript dialog may
    /// sit open for up to 120 s): further Ctrl+O and Enter are ignored,
    /// typing and Esc still work.
    pub picking: bool,
}

impl WorkspaceAddForm {
    /// Fresh overlay state on every open (the Dart `w` branch).
    pub fn reset(&mut self) {
        self.input.clear();
        self.error = None;
        self.busy = false;
        // Clearing this is what makes a picker result that lands after the
        // overlay closed (Esc / outside click / re-open) a no-op.
        self.picking = false;
    }

    pub fn insert_char(&mut self, c: char) {
        if !c.is_control() {
            self.input.push(c);
        }
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }

    /// A bracketed paste into the field, appended like any paste. The text
    /// is cleaned first: surrounding whitespace trimmed, only the first
    /// non-empty line kept, one pair of wrapping `'…'`/`"…"` quotes
    /// stripped, shell backslash escapes undone (`\ ` → space — dragging a
    /// folder from Finder into Ghostty pastes `/a/My\ Proj`), control
    /// chars dropped. Clears a stale error; a no-op while the PUT is in
    /// flight.
    pub fn insert_paste(&mut self, text: &str) {
        if self.busy {
            return;
        }
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let line = strip_wrapping_quotes(line);
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            // `\x` → `x`; a dangling trailing backslash is dropped.
            let c = if c == '\\' {
                match chars.next() {
                    Some(next) => next,
                    None => break,
                }
            } else {
                c
            };
            if !c.is_control() {
                self.input.push(c);
            }
        }
        self.error = None;
    }

    /// Ctrl+O: start a folder-picker call. `true` = the caller fires the
    /// effect; `false` while busy or already picking (one dialog at a time).
    pub fn start_pick(&mut self) -> bool {
        if self.busy || self.picking {
            return false;
        }
        self.picking = true;
        self.error = None;
        true
    }

    /// The folder-picker call settled. A pick replaces the input (the
    /// dialog chose the whole path); a cancel keeps whatever was typed;
    /// failures render inline. Ignored when no pick is in flight (the
    /// overlay was closed or re-opened meanwhile — `reset` cleared it).
    pub fn pick_settled(&mut self, outcome: PickOutcome) {
        if !self.picking {
            return;
        }
        self.picking = false;
        match outcome {
            PickOutcome::Picked(dir) => {
                self.input = dir;
                self.error = None;
            }
            PickOutcome::Cancelled => {}
            PickOutcome::Unsupported => {
                self.error =
                    Some("folder picker is macOS-only — type or paste a path".to_owned());
            }
            PickOutcome::Failed => self.error = Some("folder picker failed".to_owned()),
        }
    }

    /// Enter: validate and derive. `Some((name, dir))` starts the PUT (the
    /// caller sets an effect in motion and this form goes busy); `None`
    /// keeps the overlay open with `error` set — or is the busy-guard no-op.
    pub fn submit(
        &mut self,
        home: &str,
        existing_names: &[String],
        dir_exists: impl Fn(&str) -> bool,
    ) -> Option<(String, String)> {
        if self.busy || self.picking {
            return None;
        }
        let path = expand_tilde(&self.input, home);
        if path.is_empty() {
            self.error = Some("type a directory path".to_owned());
            return None;
        }
        if !dir_exists(&path) {
            self.error = Some(format!("no such directory: {path}"));
            return None;
        }
        let name = derive_workspace_name(&path, existing_names);
        self.busy = true;
        self.error = None;
        Some((name, path))
    }

    /// The PUT settled. A failure keeps the overlay open with the inline
    /// error; success is the caller's close-and-focus.
    pub fn settled(&mut self, error: Option<String>) {
        self.busy = false;
        self.error = error;
    }
}

/// Strip ONE pair of matching wrapping quotes (`'…'` or `"…"`).
fn strip_wrapping_quotes(s: &str) -> &str {
    for q in ['\'', '"'] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return &s[1..s.len() - 1];
        }
    }
    s
}

fn field_line(form: &WorkspaceAddForm, width: usize) -> Line<'static> {
    let mut spans = Vec::new();
    if form.input.is_empty() {
        spans.push(Span::styled(
            "~/dev/my-project",
            Style::default().fg(colors::FAINT),
        ));
    } else {
        // Show the tail when the path outgrows the field.
        let chars: Vec<char> = form.input.chars().collect();
        let visible = width.saturating_sub(1);
        let start = chars.len().saturating_sub(visible);
        spans.insert(
            0,
            Span::styled(
                chars[start..].iter().collect::<String>(),
                Style::default().fg(colors::FG),
            ),
        );
    }
    // Block cursor after the text (the field always owns the keys).
    spans.push(Span::styled(
        " ",
        Style::default().add_modifier(Modifier::REVERSED),
    ));
    Line::from(spans)
}

/// Content lines: label, field, optional error, blank, footer.
pub fn workspace_add_lines(form: &WorkspaceAddForm, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled("project directory", Style::default().fg(colors::DIM)),
        field_line(form, width),
    ];
    if let Some(error) = &form.error {
        // Red, not amber — amber is reserved for needs-input.
        lines.push(Line::styled(
            error.clone(),
            Style::default().fg(colors::ERROR),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        if form.busy {
            "registering…".to_owned()
        } else if form.picking {
            "waiting for folder picker…".to_owned()
        } else {
            // ≤ 58 cols: fits the widest modal's content (64 − border − pad).
            "Enter add · ^O browse · Esc cancel".to_owned()
        },
        Style::default().fg(colors::FAINT),
    ));
    lines
}

/// The modal rect for the current form state.
pub fn workspace_add_modal_rect(area: Rect, form: &WorkspaceAddForm) -> Rect {
    let width = modal_width(area.width, 30, 64);
    let content = 4 + u16::from(form.error.is_some());
    centered_rect(area, width, (content + 4).min(area.height))
}

/// Render the overlay; returns the modal rect (outside clicks dismiss,
/// inside clicks do nothing).
pub fn render_workspace_add(buf: &mut Buffer, area: Rect, form: &WorkspaceAddForm) -> Rect {
    let rect = workspace_add_modal_rect(area, form);
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::DIM))
        .title(Line::styled(
            " add workspace ",
            Style::default().fg(colors::FG),
        ));
    let inner = block.inner(rect);
    block.render(rect, buf);
    let padded = Rect {
        x: inner.x + 2.min(inner.width),
        y: inner.y + 1.min(inner.height),
        width: inner.width.saturating_sub(4),
        height: inner.height.saturating_sub(2),
    };
    Paragraph::new(workspace_add_lines(form, usize::from(padded.width))).render(padded, buf);
    rect
}

#[cfg(test)]
mod tests {
    //! The submit flow (port of the Dart `_submitWorkspaceAdd` behavior) and
    //! the rendered wording the p8.1 harness greps.
    use super::*;

    const HOME: &str = "/Users/me";

    fn form_with(input: &str) -> WorkspaceAddForm {
        WorkspaceAddForm {
            input: input.to_owned(),
            ..WorkspaceAddForm::default()
        }
    }

    fn none() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn empty_input_errors_inline() {
        let mut form = WorkspaceAddForm::default();
        assert_eq!(form.submit(HOME, &none(), |_| true), None);
        assert_eq!(form.error.as_deref(), Some("type a directory path"));
        assert!(!form.busy);
    }

    #[test]
    fn missing_directory_errors_with_the_expanded_path() {
        let mut form = form_with("~/dev/ghost");
        assert_eq!(form.submit(HOME, &none(), |_| false), None);
        assert_eq!(
            form.error.as_deref(),
            Some("no such directory: /Users/me/dev/ghost")
        );
    }

    #[test]
    fn a_valid_path_derives_the_name_and_goes_busy() {
        let mut form = form_with("~/dev/My Proj");
        form.error = Some("stale".to_owned());
        assert_eq!(
            form.submit(HOME, &["other".to_owned()], |_| true),
            Some(("my-proj".to_owned(), "/Users/me/dev/My Proj".to_owned()))
        );
        assert!(form.busy);
        assert_eq!(form.error, None);
        // Busy guard: a re-submit while in flight is a no-op.
        assert_eq!(form.submit(HOME, &none(), |_| true), None);
    }

    #[test]
    fn collision_suffixes_against_existing_names() {
        let mut form = form_with("/x/proj");
        assert_eq!(
            form.submit(HOME, &["proj".to_owned()], |_| true).unwrap().0,
            "proj-2"
        );
    }

    #[test]
    fn a_failed_put_keeps_the_error_and_unblocks() {
        let mut form = form_with("/x/proj");
        form.submit(HOME, &none(), |_| true);
        form.settled(Some("workspace exists".to_owned()));
        assert!(!form.busy);
        assert_eq!(form.error.as_deref(), Some("workspace exists"));
    }

    #[test]
    fn editing_and_reset() {
        let mut form = WorkspaceAddForm::default();
        form.insert_char('a');
        form.insert_char('\u{1b}'); // control chars never enter the buffer
        form.insert_char('b');
        form.backspace();
        assert_eq!(form.input, "a");
        form.error = Some("x".to_owned());
        form.busy = true;
        form.reset();
        assert_eq!(form.input, "");
        assert_eq!(form.error, None);
        assert!(!form.busy);
    }

    #[test]
    fn rendered_lines_carry_the_harness_wording() {
        let form = WorkspaceAddForm::default();
        let lines = workspace_add_lines(&form, 40);
        assert_eq!(lines[0].to_string(), "project directory");
        assert!(lines[1].to_string().contains("~/dev/my-project"), "placeholder");
        assert_eq!(
            lines[3].to_string(),
            "Enter add · ^O browse · Esc cancel"
        );
        let busy = WorkspaceAddForm {
            busy: true,
            ..WorkspaceAddForm::default()
        };
        let lines = workspace_add_lines(&busy, 40);
        assert_eq!(lines.last().unwrap().to_string(), "registering…");
        let picking = WorkspaceAddForm {
            picking: true,
            ..WorkspaceAddForm::default()
        };
        let lines = workspace_add_lines(&picking, 40);
        assert_eq!(lines.last().unwrap().to_string(), "waiting for folder picker…");
    }

    // ── paste cleanup ────────────────────────────────────────────────────

    fn pasted(existing: &str, text: &str) -> String {
        let mut form = form_with(existing);
        form.insert_paste(text);
        form.input
    }

    #[test]
    fn paste_unescapes_finder_drag_spaces() {
        assert_eq!(
            pasted("", "/Users/me/dev/My\\ Proj\\ \\(old\\) "),
            "/Users/me/dev/My Proj (old)"
        );
    }

    #[test]
    fn paste_strips_one_pair_of_wrapping_quotes() {
        assert_eq!(pasted("", "'/a/My Proj'"), "/a/My Proj");
        assert_eq!(pasted("", "\"/a/b\""), "/a/b");
        assert_eq!(pasted("", "\"\"/a\"\""), "\"/a\"", "only one pair");
        assert_eq!(pasted("", "'/a\""), "'/a\"", "mismatched quotes stay");
    }

    #[test]
    fn paste_keeps_only_the_first_non_empty_line() {
        assert_eq!(pasted("", "\r\n  \n/a/b\r\n/c/d\n"), "/a/b");
        assert_eq!(pasted("", "\n\n"), "");
    }

    #[test]
    fn paste_trims_surrounding_whitespace_and_drops_control_chars() {
        assert_eq!(pasted("", "  \t/a/b \t "), "/a/b");
        assert_eq!(pasted("", "/a\u{1b}/b\u{7}"), "/a/b");
        assert_eq!(pasted("", "/a/b\\"), "/a/b", "dangling backslash dropped");
    }

    #[test]
    fn paste_appends_to_existing_input_and_clears_the_error() {
        let mut form = form_with("~/dev/");
        form.error = Some("no such directory: x".to_owned());
        form.insert_paste("my\\ proj\n");
        assert_eq!(form.input, "~/dev/my proj");
        assert_eq!(form.error, None);
    }

    #[test]
    fn paste_is_ignored_while_busy() {
        let mut form = form_with("/a");
        form.busy = true;
        form.insert_paste("/b");
        assert_eq!(form.input, "/a");
    }

    // ── Ctrl+O folder picker ─────────────────────────────────────────────

    #[test]
    fn start_pick_guards_against_busy_and_double_picks() {
        let mut form = form_with("/typed");
        form.error = Some("stale".to_owned());
        assert!(form.start_pick());
        assert!(form.picking);
        assert_eq!(form.error, None);
        assert!(!form.start_pick(), "one dialog at a time");
        // Enter is ignored while the dialog is open.
        assert_eq!(form.submit(HOME, &none(), |_| true), None);
        assert!(!form.busy);
        let mut busy = form_with("/x");
        busy.busy = true;
        assert!(!busy.start_pick());
        assert!(!busy.picking);
    }

    #[test]
    fn a_picked_dir_replaces_the_input() {
        let mut form = form_with("/typed");
        form.start_pick();
        form.pick_settled(PickOutcome::Picked("/Users/me/dev/proj".to_owned()));
        assert!(!form.picking);
        assert_eq!(form.input, "/Users/me/dev/proj");
        assert_eq!(form.error, None);
    }

    #[test]
    fn a_cancelled_pick_keeps_the_input() {
        let mut form = form_with("/typed");
        form.start_pick();
        form.pick_settled(PickOutcome::Cancelled);
        assert!(!form.picking);
        assert_eq!(form.input, "/typed");
        assert_eq!(form.error, None);
    }

    #[test]
    fn picker_failures_render_inline() {
        let mut form = form_with("/typed");
        form.start_pick();
        form.pick_settled(PickOutcome::Unsupported);
        assert_eq!(
            form.error.as_deref(),
            Some("folder picker is macOS-only — type or paste a path")
        );
        assert_eq!(form.input, "/typed");
        form.start_pick();
        form.pick_settled(PickOutcome::Failed);
        assert_eq!(form.error.as_deref(), Some("folder picker failed"));
        assert!(!form.picking);
    }

    #[test]
    fn a_pick_result_after_reset_is_ignored() {
        let mut form = WorkspaceAddForm::default();
        form.start_pick();
        form.reset(); // Esc / outside click / re-open
        assert!(!form.picking);
        form.pick_settled(PickOutcome::Picked("/late".to_owned()));
        assert_eq!(form.input, "");
        assert_eq!(form.error, None);
    }

    #[test]
    fn error_line_renders_red_between_field_and_footer() {
        let form = WorkspaceAddForm {
            error: Some("no such directory: /tmp/nope".to_owned()),
            ..WorkspaceAddForm::default()
        };
        let lines = workspace_add_lines(&form, 40);
        assert_eq!(lines[2].to_string(), "no such directory: /tmp/nope");
        assert_eq!(lines[2].style.fg, Some(colors::ERROR));
    }

    #[test]
    fn long_input_shows_the_tail() {
        let form = form_with(&"x".repeat(60));
        let line = field_line(&form, 20).to_string();
        assert!(line.chars().count() <= 20);
    }
}
