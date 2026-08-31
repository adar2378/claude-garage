//! The `D` (shift+d) view-picker overlay (spec tui-views "Move to a group"):
//! a small modal listing every OTHER view of the focused workspace (the
//! focused session's own current view is excluded — moving a session to the
//! view it's already in is a confusing no-op) plus a trailing "new group…"
//! entry. Selecting a view moves the focused session into it;
//! selecting "new group…" switches to a text-input sub-mode (reusing the
//! `workspace_add` field pattern) whose Enter creates the group and moves
//! the session in. `j`/`k`/arrows move the selection, Enter selects, Esc
//! cancels from either mode, clicks select (runtime.rs wires the keys/clicks
//! — this module is pure content + geometry, like `triage`/`workspace_add`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::ui::layout::{centered_rect, modal_width};
use crate::ui::theme::colors;

/// The fixed trailing row every picker list ends with.
pub const NEW_GROUP_LABEL: &str = "new group…";

/// The picker's state: a selection index into the list (views + "new
/// group…"), plus the "new group…" text-input sub-mode buffer when active.
#[derive(Default)]
pub struct ViewPickerState {
    pub selected: usize,
    /// `Some` while the "new group…" text field owns the keys.
    pub new_group_input: Option<String>,
}

impl ViewPickerState {
    /// Fresh state on every open (the Dart-style `w`/`A` reset pattern).
    pub fn reset(&mut self) {
        self.selected = 0;
        self.new_group_input = None;
    }

    pub fn insert_char(&mut self, c: char) {
        if let Some(input) = self.new_group_input.as_mut() {
            if !c.is_control() {
                input.push(c);
            }
        }
    }

    pub fn backspace(&mut self) {
        if let Some(input) = self.new_group_input.as_mut() {
            input.pop();
        }
    }
}

/// The list of selectable rows: every OTHER view of the workspace (order
/// given, the focused session's own current view dropped — moving a session
/// to the view it's already in is a no-op row that only confuses), then the
/// fixed "new group…" entry.
pub fn picker_entries(view_names: &[String], current_view: &str) -> Vec<String> {
    let mut entries: Vec<String> =
        view_names.iter().filter(|name| name.as_str() != current_view).cloned().collect();
    entries.push(NEW_GROUP_LABEL.to_owned());
    entries
}

fn field_line(input: &str, width: usize) -> Line<'static> {
    let mut spans = Vec::new();
    if input.is_empty() {
        spans.push(Span::styled("group name", Style::default().fg(colors::FAINT)));
    } else {
        let chars: Vec<char> = input.chars().collect();
        let visible = width.saturating_sub(1);
        let start = chars.len().saturating_sub(visible);
        spans.push(Span::styled(
            chars[start..].iter().collect::<String>(),
            Style::default().fg(colors::FG),
        ));
    }
    spans.push(Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED)));
    Line::from(spans)
}

/// The modal's content lines: either the selectable list (+ footer) or, in
/// the "new group…" sub-mode, the text field (+ footer).
pub fn picker_lines(
    entries: &[String],
    selected: usize,
    new_group_input: Option<&str>,
    width: usize,
) -> Vec<Line<'static>> {
    if let Some(input) = new_group_input {
        return vec![
            Line::styled("new group name", Style::default().fg(colors::DIM)),
            field_line(input, width),
            Line::raw(""),
            Line::styled("Enter create · Esc cancel", Style::default().fg(colors::FAINT)),
        ];
    }
    let selected = selected.min(entries.len().saturating_sub(1));
    let mut lines: Vec<Line<'static>> = entries
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let sel = i == selected;
            let mut style = Style::default().fg(if sel { colors::FG } else { colors::DIM });
            if sel {
                style = style.add_modifier(Modifier::BOLD);
            }
            Line::from(vec![
                Span::styled(if sel { "▸ " } else { "  " }, Style::default().fg(colors::FG)),
                Span::styled(name.clone(), style),
            ])
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "j/k move · Enter select · Esc cancel",
        Style::default().fg(colors::FAINT),
    ));
    lines
}

/// The modal box rect for `n_rows` content rows (list length, or 2 for the
/// text-input sub-mode) — same border+padding shape as `triage`/`help`.
pub fn picker_modal_rect(area: Rect, n_rows: usize) -> Rect {
    let width = modal_width(area.width, 24, 60);
    let content_rows = n_rows.max(1) as u16 + 2; // rows + blank + footer
    let height = (content_rows + 4).min(area.height);
    centered_rect(area, width, height)
}

/// Render the overlay; returns the modal rect for click routing (row hits
/// resolve through `hit_targets::view_picker_row_index_at`).
pub fn render_view_picker(
    buf: &mut Buffer,
    area: Rect,
    entries: &[String],
    selected: usize,
    new_group_input: Option<&str>,
) -> Rect {
    let n_rows = if new_group_input.is_some() { 2 } else { entries.len() };
    let rect = picker_modal_rect(area, n_rows);
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::DIM))
        .title(Line::styled(" move to group ", Style::default().fg(colors::FG)));
    let inner = block.inner(rect);
    block.render(rect, buf);
    let padded = Rect {
        x: inner.x + 2.min(inner.width),
        y: inner.y + 1.min(inner.height),
        width: inner.width.saturating_sub(4),
        height: inner.height.saturating_sub(2),
    };
    Paragraph::new(picker_lines(entries, selected, new_group_input, usize::from(padded.width)))
        .render(padded, buf);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_append_the_new_group_row() {
        let names = vec!["main".to_owned(), "backend".to_owned()];
        // Neither name is the focused session's current view here, so both
        // survive the exclusion filter untouched.
        assert_eq!(picker_entries(&names, "solo"), ["main", "backend", NEW_GROUP_LABEL]);
    }

    #[test]
    fn entries_drop_the_focused_sessions_current_view() {
        // Moving a session to the view it's already in is a no-op row that
        // only confuses — it must never appear alongside the OTHER views.
        let names = vec!["main".to_owned(), "backend".to_owned(), "solo".to_owned()];
        assert_eq!(picker_entries(&names, "backend"), ["main", "solo", NEW_GROUP_LABEL]);
    }

    #[test]
    fn entries_are_just_new_group_when_the_current_view_is_the_only_one() {
        // A lone/solo view with nowhere else to go: still never a
        // zero-row modal — "new group…" is always present.
        let names = vec!["solo".to_owned()];
        assert_eq!(picker_entries(&names, "solo"), [NEW_GROUP_LABEL]);
    }

    #[test]
    fn list_lines_mark_the_selected_row() {
        let entries = picker_entries(&["main".to_owned()], "solo");
        let lines = picker_lines(&entries, 1, None, 40);
        assert_eq!(lines[0].to_string(), "  main");
        assert_eq!(lines[1].to_string(), format!("▸ {NEW_GROUP_LABEL}"));
        assert_eq!(lines[1].spans[1].style.add_modifier, Modifier::BOLD);
        assert_eq!(lines.last().unwrap().to_string(), "j/k move · Enter select · Esc cancel");
    }

    #[test]
    fn out_of_range_selection_clamps_to_the_last_row() {
        let entries = picker_entries(&["main".to_owned()], "solo");
        let lines = picker_lines(&entries, 99, None, 40);
        assert_eq!(lines[1].to_string(), format!("▸ {NEW_GROUP_LABEL}"));
    }

    #[test]
    fn new_group_input_mode_shows_the_field_and_its_own_footer() {
        let entries = picker_entries(&["main".to_owned()], "solo");
        let lines = picker_lines(&entries, 0, Some("back"), 40);
        assert_eq!(lines[0].to_string(), "new group name");
        assert!(lines[1].to_string().starts_with("back"));
        assert_eq!(lines.last().unwrap().to_string(), "Enter create · Esc cancel");
    }

    #[test]
    fn empty_input_shows_the_placeholder() {
        let entries = picker_entries(&[], "solo");
        let lines = picker_lines(&entries, 0, Some(""), 40);
        assert!(lines[1].to_string().starts_with("group name"));
    }

    #[test]
    fn state_insert_backspace_and_reset() {
        let mut state = ViewPickerState::default();
        state.insert_char('a'); // no-op: not in input mode yet
        assert_eq!(state.new_group_input, None);
        state.new_group_input = Some(String::new());
        state.insert_char('x');
        state.insert_char('\u{7f}'); // control chars never enter the buffer
        state.insert_char('y');
        assert_eq!(state.new_group_input.as_deref(), Some("xy"));
        state.backspace();
        assert_eq!(state.new_group_input.as_deref(), Some("x"));
        state.selected = 2;
        state.reset();
        assert_eq!(state.selected, 0);
        assert_eq!(state.new_group_input, None);
    }

    #[test]
    fn modal_rect_fits_and_centers() {
        let area = Rect::new(0, 0, 120, 40);
        let entries = picker_entries(&["main".to_owned(), "backend".to_owned()], "solo");
        let r = picker_modal_rect(area, entries.len());
        assert!(r.width < area.width && r.height < area.height);
        assert_eq!(r.height, 3 + 2 + 4, "3 rows + blank/footer + border/padding");
    }
}
