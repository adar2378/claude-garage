//! The quick-send overlay (`s` / `S`): a one-line prompt box that sends its
//! text to one session, or to every live session of the workspace, without
//! engaging them — answer five sessions without ever leaving the wall.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::ui::layout::{centered_rect, modal_width};
use crate::ui::theme::colors;

/// The text shown in the input row: newlines as `⏎`, tail-truncated so the
/// cursor end is always visible.
pub fn visible_input(text: &str, width: usize) -> String {
    let flat: String = text.chars().map(|c| if c == '\n' { '⏎' } else { c }).collect();
    let chars: Vec<char> = flat.chars().collect();
    if chars.len() <= width {
        return flat;
    }
    let mut out = String::from("…");
    out.extend(&chars[chars.len() - (width - 1)..]);
    out
}

pub fn render_quick_send(buf: &mut Buffer, area: Rect, scope: &str, text: &str) -> Rect {
    let width = modal_width(area.width, 40, 100);
    let rect = centered_rect(area, width, 7.min(area.height));
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::DIM))
        .title(Line::styled(format!(" send to {scope} "), Style::default().fg(colors::FG)));
    let inner = block.inner(rect);
    block.render(rect, buf);
    let padded = Rect {
        x: inner.x + 2.min(inner.width),
        y: inner.y + 1.min(inner.height),
        width: inner.width.saturating_sub(4),
        height: inner.height.saturating_sub(2),
    };
    let input_width = usize::from(padded.width).saturating_sub(3);
    let lines = vec![
        Line::from(vec![
            Span::styled("› ", Style::default().fg(colors::AMBER)),
            Span::styled(visible_input(text, input_width), Style::default().fg(colors::FG)),
            Span::styled("▏", Style::default().fg(colors::FG)),
        ]),
        Line::raw(""),
        Line::styled(
            "Enter send · Esc cancel · Ctrl+U clear · paste works",
            Style::default().fg(colors::FAINT),
        ),
    ];
    Paragraph::new(lines).render(padded, buf);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_shown_whole_with_newlines_flattened() {
        assert_eq!(visible_input("a\nb", 10), "a⏎b");
    }

    #[test]
    fn long_text_keeps_the_tail() {
        assert_eq!(visible_input("abcdefghij", 5), "…ghij");
    }
}
