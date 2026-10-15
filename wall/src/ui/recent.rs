//! The recent-sessions overlay (`H`): conversations that were closed with
//! `x x`, newest first. Enter reopens one as a new session via
//! `claude --resume`; `j`/`k` move, Esc closes.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::api::models::RecentInfo;
use crate::ui::layout::{centered_rect, fit, modal_width};
use crate::ui::theme::{colors, format_elapsed};

fn lines(rows: &[RecentInfo], selected: usize, loading: bool, width: usize, now_ms: i64) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if rows.is_empty() {
        out.push(Line::styled(
            if loading { "loading…" } else { "no recent sessions yet — they appear here after x x" },
            Style::default().fg(colors::DIM),
        ));
    } else {
        let selected = selected.min(rows.len() - 1);
        for (i, r) in rows.iter().enumerate() {
            let on = i == selected;
            let ago = format_elapsed(r.ended_at, now_ms);
            let identity = format!("{}/{}", r.workspace, r.display_name());
            let mut style = Style::default().fg(if on { colors::FG } else { colors::DIM });
            if on {
                style = style.add_modifier(Modifier::BOLD);
            }
            out.push(Line::from(vec![
                Span::styled(if on { "▸ " } else { "  " }, Style::default().fg(colors::FG)),
                Span::styled(fit(&identity, width.saturating_sub(4 + ago.chars().count() + 6)), style),
                Span::styled(format!("  {ago} ago"), Style::default().fg(colors::FAINT)),
            ]));
        }
    }
    out.push(Line::raw(""));
    out.push(Line::styled(
        "j/k move · Enter reopen · Esc close",
        Style::default().fg(colors::FAINT),
    ));
    out
}

/// Render the overlay; returns the modal rect.
pub fn render_recent(
    buf: &mut Buffer,
    area: Rect,
    rows: &[RecentInfo],
    selected: usize,
    loading: bool,
    now_ms: i64,
) -> Rect {
    let width = modal_width(area.width, 30, 90);
    let content_rows = rows.len().clamp(1, 20) as u16 + 2;
    let rect = centered_rect(area, width, (content_rows + 4).min(area.height));
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::DIM))
        .title(Line::styled(" recent sessions ", Style::default().fg(colors::FG)));
    let inner = block.inner(rect);
    block.render(rect, buf);
    let padded = Rect {
        x: inner.x + 2.min(inner.width),
        y: inner.y + 1.min(inner.height),
        width: inner.width.saturating_sub(4),
        height: inner.height.saturating_sub(2),
    };
    let text_width = usize::from(rect.width.saturating_sub(6));
    Paragraph::new(lines(rows, selected, loading, text_width, now_ms)).render(padded, buf);
    rect
}
