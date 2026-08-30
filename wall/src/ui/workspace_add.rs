//! The `w` add-workspace overlay (port of
//! `tui/lib/ui/workspace_add_overlay.dart` + the submit logic from
//! `bin/garage_tui.dart` — spec tui-key-routing "p8.1"): a centered modal
//! with one text field for a directory path. Enter submits (`~` expansion,
//! client-side dir-exists validation, web-UI-style name derivation, then the
//! PUT effect); Esc cancels; validation/daemon errors render inline and keep
//! the overlay open.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::state::workspace_form::{derive_workspace_name, expand_tilde};
use crate::ui::layout::{centered_rect, modal_width};
use crate::ui::theme::colors;

/// The overlay's state: one text buffer plus the inline error and the
/// in-flight guard. Pure (IO is injected) so the whole submit flow
/// unit-tests without a filesystem or daemon.
#[derive(Default)]
pub struct WorkspaceAddForm {
    pub input: String,
    pub error: Option<String>,
    pub busy: bool,
}

impl WorkspaceAddForm {
    /// Fresh overlay state on every open (the Dart `w` branch).
    pub fn reset(&mut self) {
        self.input.clear();
        self.error = None;
        self.busy = false;
    }

    pub fn insert_char(&mut self, c: char) {
        if !c.is_control() {
            self.input.push(c);
        }
    }

    pub fn backspace(&mut self) {
        self.input.pop();
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
        if self.busy {
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
        } else {
            "Enter add · Esc cancel · name derives from the folder".to_owned()
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
            "Enter add · Esc cancel · name derives from the folder"
        );
        let busy = WorkspaceAddForm {
            busy: true,
            ..WorkspaceAddForm::default()
        };
        let lines = workspace_add_lines(&busy, 40);
        assert_eq!(lines.last().unwrap().to_string(), "registering…");
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
