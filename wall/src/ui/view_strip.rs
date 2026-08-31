//! The per-workspace view strip (spec tui-views "View strip and group
//! frame"): a one-line strip rendered directly above the grid, only when the
//! focused workspace currently has 2+ views — names left to right, the
//! focused view emphasized (bright + underlined), an amber dot on any view
//! holding a needs-input session. With a single view nothing renders here at
//! all (the caller never even reserves the row — see `ui::layout`).

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::state::views::ViewSummary;
use crate::ui::theme::colors;

pub struct ViewStripLine {
    pub line: Line<'static>,
    /// Column range (strip-local) of each view's clickable span, in the same
    /// order as the `views` slice passed to [`view_strip_line`] — the mouse
    /// router uses these verbatim (spec: "view-strip click focuses that
    /// view").
    pub spans: Vec<Range<u16>>,
}

/// Build the strip's line for `width` columns. Returns an empty line (no
/// spans) for fewer than 2 views — the spec's "no strip renders" case — so a
/// caller that always calls this can never accidentally paint a one-view
/// strip.
pub fn view_strip_line(views: &[ViewSummary], focused: &str, width: u16) -> ViewStripLine {
    if views.len() < 2 {
        return ViewStripLine { line: Line::default(), spans: Vec::new() };
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut ranges = Vec::new();
    for v in views {
        let start: usize = spans.iter().map(Span::width).sum();
        let is_focused = v.name == focused;
        let mut style = Style::default().fg(if is_focused { colors::FG } else { colors::DIM });
        if is_focused {
            style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
        spans.push(Span::styled(format!(" {} ", v.name), style));
        if v.needs_input_count > 0 {
            spans.push(Span::styled("●", Style::default().fg(colors::AMBER)));
        }
        let end: usize = spans.iter().map(Span::width).sum();
        ranges.push(start.min(usize::from(width)) as u16..end.min(usize::from(width)) as u16);
    }
    ViewStripLine { line: Line::from(spans), spans: ranges }
}

/// Render the strip into its 1-row rect (a no-op for fewer than 2 views, or
/// a zero-height rect); returns each view's absolute column range, in the
/// same order as `views`, for click routing.
pub fn render_view_strip(
    buf: &mut Buffer,
    rect: Rect,
    views: &[ViewSummary],
    focused: &str,
) -> Vec<Range<u16>> {
    if rect.height == 0 || views.len() < 2 {
        return Vec::new();
    }
    let built = view_strip_line(views, focused, rect.width);
    Paragraph::new(built.line)
        .style(Style::default().bg(Color::Black))
        .render(rect, buf);
    built.spans.into_iter().map(|r| rect.x + r.start..rect.x + r.end).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(name: &str, ids: &[&str], needs_input_count: usize) -> ViewSummary {
        ViewSummary {
            name: name.to_owned(),
            session_ids: ids.iter().map(|s| (*s).to_owned()).collect(),
            needs_input_count,
        }
    }

    #[test]
    fn renders_nothing_for_fewer_than_two_views() {
        let one = [view("main", &["a"], 0)];
        let built = view_strip_line(&one, "main", 80);
        assert_eq!(built.line.to_string(), "");
        assert!(built.spans.is_empty());

        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 1));
        let clicks = render_view_strip(&mut buf, Rect::new(0, 0, 80, 1), &one, "main");
        assert!(clicks.is_empty());
    }

    #[test]
    fn names_render_left_to_right_focused_view_emphasized() {
        let views = [view("main", &["a"], 0), view("backend", &["b", "c"], 0)];
        let built = view_strip_line(&views, "backend", 80);
        assert_eq!(built.line.to_string(), " main  backend ");
        assert_eq!(built.spans.len(), 2);

        let main_span = &built.line.spans[0];
        assert!(!main_span.style.add_modifier.contains(Modifier::BOLD));
        let backend_span = &built.line.spans[1];
        assert!(backend_span.style.add_modifier.contains(Modifier::BOLD));
        assert!(backend_span.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn a_view_with_a_needs_input_session_gets_an_amber_dot() {
        let views = [view("main", &["a"], 0), view("backend", &["b"], 1)];
        let built = view_strip_line(&views, "main", 80);
        assert_eq!(built.line.to_string(), " main  backend ●");
        let dot = built.line.spans.last().unwrap();
        assert_eq!(dot.style.fg, Some(colors::AMBER));
    }

    #[test]
    fn click_ranges_match_the_rendered_text() {
        let views = [view("main", &["a"], 0), view("backend", &["b"], 1)];
        let built = view_strip_line(&views, "main", 80);
        let text = built.line.to_string();
        for r in &built.spans {
            let slice: String = text
                .chars()
                .skip(usize::from(r.start))
                .take(usize::from(r.end - r.start))
                .collect();
            assert!(!slice.trim().is_empty(), "{text} / {r:?}");
        }
    }

    #[test]
    fn render_returns_absolute_column_ranges() {
        let views = [view("main", &["a"], 0), view("backend", &["b"], 0)];
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        let rect = Rect::new(5, 3, 80, 1);
        let clicks = render_view_strip(&mut buf, rect, &views, "main");
        assert_eq!(clicks.len(), 2);
        assert_eq!(clicks[0].start, rect.x);
    }
}
