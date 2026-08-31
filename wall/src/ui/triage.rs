//! The triage queue overlay (port of `tui/lib/ui/triage_overlay.dart` —
//! spec tui-triage "Triage queue overlay"): every needs-input session across
//! workspaces, sorted by waiting time (longest first), each row showing the
//! amber glyph, workspace/label, waiting duration, and the daemon's
//! notification message when present (dim, truncated). `j`/`k` wrap the
//! selection, Enter jump-engages, Esc closes; the empty state reads
//! "nothing needs you".

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::state::wall_state::WallSession;
use crate::ui::layout::{centered_rect, modal_width};
use crate::ui::theme::{colors, format_elapsed};

/// Queue rows: needs-input sessions only, longest-waiting first (smallest
/// `since`). Stable for ties, and a `None` `since` (unknown transition time)
/// sorts after every known one — mirroring `jump_target`'s "a null since
/// never beats a known one".
pub fn triage_queue_rows(sessions: &[WallSession]) -> Vec<&WallSession> {
    let mut rows: Vec<&WallSession> = sessions.iter().filter(|s| s.needs_input()).collect();
    rows.sort_by_key(|s| s.since.map_or((1, 0), |since| (0, since)));
    rows
}

/// j/k selection movement with wrap-around. Degenerate lengths pin to 0.
pub fn wrap_selection(current: usize, delta: i32, length: usize) -> usize {
    if length == 0 {
        return 0;
    }
    let current = current.min(length - 1) as i32;
    (current + delta).rem_euclid(length as i32) as usize
}

/// Truncate to `width` cells with a trailing ellipsis.
fn fit(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }
    let mut out: String = chars[..width - 1].iter().collect();
    out.push('…');
    out
}

/// The modal box rect for `n` queue rows (0 renders the one-line empty
/// state). Content = rows + blank + footer, wrapped in border + padding.
pub fn triage_modal_rect(area: Rect, n_rows: usize) -> Rect {
    let width = modal_width(area.width, 24, 78);
    let content_rows = n_rows.max(1) as u16 + 2; // rows/empty + blank + footer
    let height = (content_rows + 4).min(area.height);
    centered_rect(area, width, height)
}

fn row_line(s: &WallSession, selected: bool, width: usize, now_ms: i64) -> Line<'static> {
    let identity = format!("{}/{}", s.workspace, s.label);
    let waiting = format_elapsed(s.since, now_ms);
    // '▸ ● ' prefix (4) + identity + 2 spaces + waiting + 2 spaces.
    let head = 4 + identity.chars().count() + 2 + waiting.chars().count();
    let message_width = width.saturating_sub(head + 2);
    let mut identity_style = Style::default().fg(if selected { colors::FG } else { colors::DIM });
    if selected {
        identity_style = identity_style.add_modifier(Modifier::BOLD);
    }
    let mut spans = vec![
        Span::styled(
            if selected { "▸ " } else { "  " },
            Style::default().fg(colors::FG),
        ),
        Span::styled("● ", Style::default().fg(colors::AMBER)),
        Span::styled(fit(&identity, width.saturating_sub(4)), identity_style),
        Span::styled(format!("  {waiting}"), Style::default().fg(colors::AMBER)),
    ];
    if let Some(message) = &s.message {
        let fitted = fit(message, message_width);
        if !fitted.is_empty() {
            spans.push(Span::styled(
                format!("  {fitted}"),
                Style::default().fg(colors::FAINT),
            ));
        }
    }
    // Subtitle (spec tui-wall "Auto-subtitle in the tile bar": "The triage
    // queue row MAY show the same subtitle dim when present"), lowest
    // priority — fitted into whatever's left after identity/waiting/message.
    if let Some(title) = s.title.as_deref() {
        if title != s.label {
            let used: usize = spans.iter().map(Span::width).sum();
            let remaining = width.saturating_sub(used + 2);
            let fitted = fit(title, remaining);
            if !fitted.is_empty() {
                spans.push(Span::styled(
                    format!("  {fitted}"),
                    Style::default().fg(colors::DIM),
                ));
            }
        }
    }
    Line::from(spans)
}

/// The modal's content lines (rows + blank + footer) — split out so the
/// text is testable without a buffer.
pub fn triage_lines(
    rows: &[&WallSession],
    selected: usize,
    width: usize,
    now_ms: i64,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if rows.is_empty() {
        lines.push(Line::styled(
            "nothing needs you",
            Style::default().fg(colors::DIM),
        ));
    } else {
        let selected = selected.min(rows.len() - 1);
        for (i, s) in rows.iter().enumerate() {
            lines.push(row_line(s, i == selected, width, now_ms));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "j/k move · Enter jump · Esc close",
        Style::default().fg(colors::FAINT),
    ));
    lines
}

/// Render the overlay; returns the modal rect for click routing (row hits
/// resolve through `triage_row_index_at` with the modal-local row).
pub fn render_triage(
    buf: &mut Buffer,
    area: Rect,
    rows: &[&WallSession],
    selected: usize,
    now_ms: i64,
) -> Rect {
    let rect = triage_modal_rect(area, rows.len());
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::DIM))
        .title(Line::styled(" triage queue ", Style::default().fg(colors::FG)));
    let inner = block.inner(rect);
    block.render(rect, buf);
    let padded = Rect {
        x: inner.x + 2.min(inner.width),
        y: inner.y + 1.min(inner.height),
        width: inner.width.saturating_sub(4),
        height: inner.height.saturating_sub(2),
    };
    let text_width = usize::from(rect.width.saturating_sub(6));
    Paragraph::new(triage_lines(rows, selected, text_width, now_ms)).render(padded, buf);
    rect
}

#[cfg(test)]
mod tests {
    //! Port of `tui/test/triage_overlay_test.dart` — row ordering, wrap
    //! selection, row text with message truncation, the empty state.
    use super::*;
    use crate::ui::hit_targets::{triage_row_index_at, TRIAGE_MODAL_ROW_OFFSET};

    fn session(label: &str, status: &str, since: Option<i64>) -> WallSession {
        WallSession {
            id: format!("garage/ws/{label}"),
            workspace: "ws".to_owned(),
            label: label.to_owned(),
            dir: None,
            status: status.to_owned(),
            since,
            message: None,
            branch: None,
            worktree: false,
            title: None,
        }
    }

    #[test]
    fn rows_are_blocked_only_longest_waiting_first() {
        let sessions = vec![
            session("young", "needs-input", Some(5000)),
            session("busy", "working", Some(1)),
            session("old", "needs-input", Some(100)),
        ];
        let rows = triage_queue_rows(&sessions);
        let labels: Vec<&str> = rows.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["old", "young"]);
    }

    #[test]
    fn null_since_sorts_last_ties_stay_stable() {
        let sessions = vec![
            session("unknown", "needs-input", None),
            session("a", "needs-input", Some(50)),
            session("b", "needs-input", Some(50)),
        ];
        let rows = triage_queue_rows(&sessions);
        let labels: Vec<&str> = rows.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["a", "b", "unknown"]);
    }

    #[test]
    fn wrap_selection_wraps_both_ways_and_pins_degenerates() {
        assert_eq!(wrap_selection(0, 1, 3), 1);
        assert_eq!(wrap_selection(2, 1, 3), 0);
        assert_eq!(wrap_selection(0, -1, 3), 2);
        assert_eq!(wrap_selection(0, 1, 0), 0);
        assert_eq!(wrap_selection(9, 1, 3), 0, "out-of-range current clamps first");
    }

    #[test]
    fn row_text_shows_identity_waiting_and_message() {
        let mut s = session("api-fix", "needs-input", Some(0));
        s.message = Some("Claude needs your permission to use Bash".to_owned());
        let line = row_line(&s, true, 78, 240_000);
        assert_eq!(
            line.to_string(),
            "▸ ● ws/api-fix  4:00  Claude needs your permission to use Bash"
        );
        let unselected = row_line(&s, false, 78, 240_000);
        assert!(unselected.to_string().starts_with("  ● ws/api-fix"));
    }

    #[test]
    fn long_messages_truncate_with_an_ellipsis() {
        let mut s = session("x", "needs-input", Some(0));
        s.message = Some("abcdefghijklmnopqrstuvwxyz".repeat(4));
        let line = row_line(&s, false, 40, 0).to_string();
        assert!(line.chars().count() <= 40, "{line}");
        assert!(line.ends_with('…'), "{line}");
    }

    #[test]
    fn row_shows_the_subtitle_dim_when_present_and_distinct_from_the_label() {
        let mut s = session("api-fix", "needs-input", Some(0));
        s.title = Some("✳ writing tests".to_owned());
        let line = row_line(&s, false, 78, 240_000);
        assert!(line.to_string().ends_with("✳ writing tests"), "{line}");
        let subtitle_span = line.spans.last().unwrap();
        assert_eq!(subtitle_span.style.fg, Some(colors::DIM));
    }

    #[test]
    fn row_omits_a_title_matching_the_label() {
        let mut s = session("api-fix", "needs-input", Some(0));
        s.title = Some("api-fix".to_owned());
        let line = row_line(&s, false, 78, 240_000);
        assert!(!line.to_string().contains("api-fix  api-fix"));
    }

    #[test]
    fn empty_queue_says_nothing_needs_you() {
        let lines = triage_lines(&[], 0, 60, 0);
        assert_eq!(lines[0].to_string(), "nothing needs you");
        assert_eq!(lines[2].to_string(), "j/k move · Enter jump · Esc close");
    }

    #[test]
    fn modal_geometry_matches_the_click_offset() {
        // First queue row renders at modal-local row TRIAGE_MODAL_ROW_OFFSET
        // (border 1 + vertical padding 1) — the pure mapping's contract.
        assert_eq!(TRIAGE_MODAL_ROW_OFFSET, 2);
        let rect = triage_modal_rect(Rect::new(0, 0, 100, 40), 3);
        assert_eq!(rect.width, 78);
        assert_eq!(rect.height, 3 + 2 + 4, "rows + blank/footer + border/padding");
        assert_eq!(triage_row_index_at(2, 3), Some(0));
        assert_eq!(triage_row_index_at(4, 3), Some(2));
        assert_eq!(triage_row_index_at(5, 3), None, "blank line");
    }
}
