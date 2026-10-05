//! The left workspace rail (port of `tui/lib/ui/rail.dart` — spec tui-wall
//! "Rail, strip, and salience ladder" + p8.3 "rail focus marker"):
//! workspaces in salience order with their sessions — glyph, label, elapsed
//! — amber rows exclusively for needs-input, per-workspace blocked counts,
//! non-gridded sessions dimmed, and a reserved-column `▸` marker on the
//! FOCUSED session's row. The marker column exists on every session row, so
//! rows never shift as focus moves and the one-line-per-row click mapping
//! (`rail_target_at`) is unchanged.

use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::state::salience::WorkspaceGroup;
use crate::state::wall_state::{WallSession, WallState};
use crate::ui::theme::{colors, elapsed_for, glyph_for, status_color};

/// One rendered line per rail row: each group's optional family label, its
/// header, then its sessions — the exact rows `rail_target_at` maps clicks against.
pub fn rail_lines(state: &WallState, now_ms: i64) -> Vec<Line<'static>> {
    let gridded: HashSet<&str> = state
        .gridded_session_ids
        .iter()
        .map(String::as_str)
        .collect();
    let mut lines = Vec::new();
    for (i, group) in state.groups.iter().enumerate() {
        let focused_ws = state.focused_workspace.as_deref() == Some(group.name.as_str());
        if let Some(label) = &group.family_label {
            lines.push(Line::styled(
                format!(" {label}"),
                Style::default().fg(colors::FAINT),
            ));
        }
        lines.push(workspace_row(group, i, focused_ws));
        for s in &group.sessions {
            lines.push(session_row(
                s,
                focused_ws && gridded.contains(s.id.as_str()),
                focused_ws && state.focused_session_id.as_deref() == Some(s.id.as_str()),
                now_ms,
            ));
        }
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            " no workspaces",
            Style::default().fg(colors::FAINT),
        ));
    }
    lines
}

fn workspace_row(group: &WorkspaceGroup, index: usize, focused: bool) -> Line<'static> {
    let mut name_style = Style::default().fg(if focused { colors::FG } else { colors::DIM });
    if focused {
        name_style = name_style.add_modifier(Modifier::BOLD);
    }
    let mut spans = vec![
        Span::styled(format!(" {} ", index + 1), Style::default().fg(colors::DIM)),
        Span::styled(group.name.clone(), name_style),
    ];
    let blocked = group.sessions.iter().filter(|s| s.needs_input()).count();
    if blocked > 0 {
        spans.push(Span::styled(
            format!("  ● {blocked}"),
            Style::default().fg(colors::AMBER),
        ));
    }
    Line::from(spans)
}

fn session_row(s: &WallSession, gridded: bool, focused: bool, now_ms: i64) -> Line<'static> {
    // Amber row exclusively for needs-input; everything else follows the
    // status ladder, dimmed further when the session has no tile. The
    // focused session carries a `▸` marker plus a bright/bold label — never
    // amber (amber stays exclusive to needs-input, whose label keeps its hue
    // and just gains bold). The marker column is always reserved so rows
    // never shift as focus moves.
    let glyph_color = if gridded {
        status_color(&s.status, s.since, now_ms)
    } else {
        colors::FAINT
    };
    let label_color = if s.needs_input() {
        colors::AMBER
    } else if focused {
        colors::FG
    } else if gridded {
        colors::DIM
    } else {
        colors::FAINT
    };
    let mut label_style = Style::default().fg(label_color);
    if focused {
        label_style = label_style.add_modifier(Modifier::BOLD);
    }
    let mut spans = vec![
        Span::styled(
            if focused { " ▸ " } else { "   " },
            Style::default().fg(colors::FG).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} ", glyph_for(&s.status)),
            Style::default().fg(glyph_color),
        ),
        // Spec tui-wall "Title as display name": the live title when there
        // is one, else the label. Paragraph clips at the rail edge.
        Span::styled(s.display_name().to_owned(), label_style),
    ];
    if let Some(elapsed) = elapsed_for(&s.status, s.since, now_ms) {
        spans.push(Span::styled(
            format!(" {elapsed}"),
            Style::default().fg(if s.needs_input() {
                colors::AMBER
            } else {
                colors::FAINT
            }),
        ));
    }
    Line::from(spans)
}

/// Render the rail with its right border. Rows stay aligned with `rect`'s
/// rows (only a RIGHT border — no top inset), so the click router can map
/// `row - rect.y` straight through `rail_target_at`.
pub fn render_rail(buf: &mut Buffer, rect: Rect, state: &WallState, now_ms: i64) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(colors::FAINT));
    let inner = block.inner(rect);
    block.render(rect, buf);
    Paragraph::new(rail_lines(state, now_ms)).render(inner, buf);
}

#[cfg(test)]
mod tests {
    //! Rail row content (port of `tui/test/rail_test.dart` semantics): the
    //! reserved marker column, salience colors, blocked counts, dimmed
    //! overflow rows.
    use super::*;
    use crate::api::models::{SessionInfo, WorkspaceInfo};
    use crate::state::store::WallStore;

    fn ws(name: &str) -> WorkspaceInfo {
        WorkspaceInfo {
            name: name.to_owned(),
            dir: Some(format!("/repos/{name}/{name}")),
            branch: None,
        }
    }

    fn si(workspace: &str, label: &str, status: &str, since: Option<i64>) -> SessionInfo {
        SessionInfo {
            id: format!("garage/{workspace}/{label}"),
            workspace: workspace.to_owned(),
            label: label.to_owned(),
            dir: None,
            attached: false,
            status: status.to_owned(),
            since,
            message: None,
            branch: None,
            restorable: status == "restorable",
            title: None,
            context: None,
        }
    }

    fn texts(lines: &[Line]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn one_line_per_row_headers_then_sessions() {
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![ws("alpha"), ws("beta")]);
        store.sessions_fetched(vec![
            si("alpha", "one", "working", Some(0)),
            si("alpha", "two", "idle", None),
            si("beta", "other", "idle", None),
        ]);
        let rows = texts(&rail_lines(store.state(), 60_000));
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0], " 1 alpha");
        assert_eq!(rows[1], " ▸ ◐ one 1:00", "focused session carries the marker");
        assert_eq!(rows[2], "   ○ two", "reserved marker column on every row");
        assert_eq!(rows[3], " 2 beta");
        assert_eq!(rows[4], "   ○ other");
    }

    #[test]
    fn blocked_count_on_the_workspace_header() {
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![ws("a")]);
        store.sessions_fetched(vec![
            si("a", "b1", "needs-input", Some(0)),
            si("a", "b2", "needs-input", Some(0)),
            si("a", "w", "working", Some(0)),
        ]);
        let rows = texts(&rail_lines(store.state(), 0));
        assert_eq!(rows[0], " 1 a  ● 2");
    }

    #[test]
    fn needs_input_rows_are_amber_marker_never_is() {
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![ws("a")]);
        store.sessions_fetched(vec![si("a", "blocked", "needs-input", Some(0))]);
        let line = &rail_lines(store.state(), 60_000)[1];
        assert_eq!(line.to_string(), " ▸ ● blocked 1:00");
        // marker span is fg, glyph/label/elapsed spans amber.
        assert_eq!(line.spans[0].style.fg, Some(colors::FG));
        assert_eq!(line.spans[1].style.fg, Some(colors::AMBER));
        assert_eq!(line.spans[2].style.fg, Some(colors::AMBER));
        assert_eq!(line.spans[3].style.fg, Some(colors::AMBER));
    }

    #[test]
    fn non_gridded_sessions_render_faint() {
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![ws("a")]);
        store.sessions_fetched((1..=7).map(|i| si("a", &format!("s{i}"), "working", Some(0))).collect());
        let lines = rail_lines(store.state(), 0);
        let overflow = &lines[7]; // header + 6 gridded rows before it
        assert!(overflow.to_string().contains("s7"));
        assert_eq!(overflow.spans[1].style.fg, Some(colors::FAINT), "glyph dimmed");
        assert_eq!(overflow.spans[2].style.fg, Some(colors::FAINT), "label dimmed");
    }

    #[test]
    fn family_label_row_sits_above_the_first_member_only() {
        let mut store = WallStore::new();
        let sib = |name: &str| WorkspaceInfo {
            name: name.to_owned(),
            dir: Some(format!("/work/elite-traders/{name}")),
            branch: None,
        };
        store.workspaces_fetched(vec![sib("et-a"), sib("et-b"), ws("solo")]);
        let lines = rail_lines(store.state(), 0);
        let rows = texts(&lines);
        assert_eq!(rows, [" elite-traders", " 1 et-a", " 2 et-b", " 3 solo"]);
        assert_eq!(lines[0].style.fg, Some(colors::FAINT));
    }

    #[test]
    fn empty_rail_names_the_state() {
        let store = WallStore::new();
        let rows = texts(&rail_lines(store.state(), 0));
        assert_eq!(rows, [" no workspaces"]);
    }
}
