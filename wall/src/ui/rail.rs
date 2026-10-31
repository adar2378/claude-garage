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

use crate::slots::{self, ClickTarget, SlotClickRegion, SlotCtx, SlotId, SlotItem, SlotRegistry};
use crate::state::salience::WorkspaceGroup;
use crate::state::wall_state::{WallSession, WallState};
use crate::ui::theme::{colors, elapsed_for, glyph_for, status_color, status_color_fading};

/// A clickable slot item on one rail row: `cols` are rail-local columns.
#[derive(Debug, PartialEq, Eq)]
pub struct RailClick {
    pub row: u16,
    pub cols: std::ops::Range<u16>,
    pub target: ClickTarget,
    pub slot: SlotId,
    pub session: Option<String>,
    pub workspace: Option<String>,
}

/// What [`rail_render`] produces besides the lines.
#[derive(Default)]
pub struct RailExtras {
    pub clicks: Vec<RailClick>,
    /// Styles laid over a whole row (`rail.extra_rows` backgrounds).
    pub row_styles: Vec<(u16, Style)>,
}

/// One rendered line per rail row: each group's optional family label, its
/// header, then its sessions — the exact rows `rail_target_at` maps clicks against.
pub fn rail_lines(state: &WallState, slots: &SlotRegistry, now_ms: i64) -> Vec<Line<'static>> {
    rail_render(state, slots, now_ms, false).0
}

/// [`rail_lines`] with the `done` glyph fading across its 2 minutes when
/// `fade_done` (motion on).
pub fn rail_lines_fading(
    state: &WallState,
    slots: &SlotRegistry,
    now_ms: i64,
    fade_done: bool,
) -> Vec<Line<'static>> {
    rail_render(state, slots, now_ms, fade_done).0
}

/// The rail's lines plus the clickable slot items painted on them.
pub fn rail_render(
    state: &WallState,
    slots: &SlotRegistry,
    now_ms: i64,
    fade_done: bool,
) -> (Vec<Line<'static>>, RailExtras) {
    let mut extras = RailExtras::default();
    let clicks = &mut extras.clicks;
    let gridded: HashSet<&str> = state
        .gridded_session_ids
        .iter()
        .map(String::as_str)
        .collect();
    let mut lines = Vec::new();
    for (i, group) in state.groups.iter().enumerate() {
        let focused_ws = state.focused_workspace.as_deref() == Some(group.name.as_str());
        // Rows above the workspace row come from the `rail.header` slot;
        // `rail_target_at` counts them through the same call.
        for item in header_items(state, slots, group) {
            if let Some(target) = item.on_click.clone() {
                clicks.push(RailClick {
                    row: lines.len() as u16,
                    cols: 0..item.width().min(usize::from(u16::MAX)) as u16,
                    target,
                    slot: SlotId::RailHeader,
                    session: None,
                    workspace: Some(group.name.clone()),
                });
            }
            lines.push(Line::from(item.text));
        }
        lines.push(workspace_row(group, i, focused_ws));
        for s in &group.sessions {
            let mut line = session_row(
                s,
                focused_ws && gridded.contains(s.id.as_str()),
                focused_ws && state.focused_session_id.as_deref() == Some(s.id.as_str()),
                now_ms,
                fade_done,
            );
            // `rail.row`: plugin items appended after the elapsed time. No
            // provider, no change to the line.
            let row_ctx = SlotCtx {
                state: Some(state),
                session: Some(s),
                group: Some(group),
                ..SlotCtx::default()
            };
            let items = slots.items(SlotId::RailRow, &row_ctx);
            if !items.is_empty() {
                let laid = slots::layout(items, None);
                let offset = line.width();
                for (cols, target) in laid.clicks(offset, usize::from(u16::MAX)) {
                    clicks.push(RailClick {
                        row: lines.len() as u16,
                        cols,
                        target,
                        slot: SlotId::RailRow,
                        session: Some(s.id.clone()),
                        workspace: Some(group.name.clone()),
                    });
                }
                line.spans.extend(laid.spans);
            }
            lines.push(line);
        }
        // `rail.extra_rows`: whole rows after the sessions. A click anywhere
        // on the row runs the item's target; `about_session` scopes it.
        for item in extra_row_items(state, slots, group) {
            if let Some(target) = item.on_click.clone() {
                let session = target.session.as_deref().map(str::to_owned);
                clicks.push(RailClick {
                    row: lines.len() as u16,
                    cols: 0..u16::MAX,
                    target,
                    slot: SlotId::RailExtraRows,
                    session,
                    workspace: Some(group.name.clone()),
                });
            }
            if let Some(style) = item.row_style {
                extras.row_styles.push((lines.len() as u16, style));
            }
            lines.push(Line::from(item.text));
        }
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            " no workspaces",
            Style::default().fg(colors::FAINT),
        ));
    }
    (lines, extras)
}

/// The `rail.header` items above a group's workspace row, one rail row each.
/// The renderer and the click router both ask here, so a plugin's header row
/// can never shift clicks.
pub fn header_items(state: &WallState, slots: &SlotRegistry, group: &WorkspaceGroup) -> Vec<SlotItem> {
    let ctx = SlotCtx {
        state: Some(state),
        group: Some(group),
        ..SlotCtx::default()
    };
    slots.items(SlotId::RailHeader, &ctx)
}

/// The `rail.extra_rows` items under a group's sessions, one rail row each.
/// The renderer and the click router both ask here, so the row maths cannot
/// drift.
pub fn extra_row_items(state: &WallState, slots: &SlotRegistry, group: &WorkspaceGroup) -> Vec<SlotItem> {
    let ctx = SlotCtx {
        state: Some(state),
        group: Some(group),
        ..SlotCtx::default()
    };
    slots.items(SlotId::RailExtraRows, &ctx)
}

/// The `rail.header` core provider: the p18 family label row above the first
/// member of a worktree family.
pub fn family_label_provider(ctx: &SlotCtx) -> Vec<SlotItem> {
    let Some(label) = ctx.group.and_then(|g| g.family_label.as_ref()) else {
        return Vec::new();
    };
    vec![SlotItem::new(
        vec![Span::styled(
            format!(" {label}"),
            Style::default().fg(colors::FAINT),
        )],
        0,
    )]
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

fn session_row(
    s: &WallSession,
    gridded: bool,
    focused: bool,
    now_ms: i64,
    fade_done: bool,
) -> Line<'static> {
    // Amber row exclusively for needs-input; everything else follows the
    // status ladder, dimmed further when the session has no tile. The
    // focused session carries a `▸` marker plus a bright/bold label — never
    // amber (amber stays exclusive to needs-input, whose label keeps its hue
    // and just gains bold). The marker column is always reserved so rows
    // never shift as focus moves.
    let glyph_color = if gridded && fade_done {
        status_color_fading(&s.status, s.since, now_ms)
    } else if gridded {
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
///
/// Returns the click regions of the clickable slot items it painted,
/// clipped to the rail's inner area.
pub fn render_rail(
    buf: &mut Buffer,
    rect: Rect,
    state: &WallState,
    slots: &SlotRegistry,
    now_ms: i64,
) -> Vec<SlotClickRegion> {
    render_rail_fading(buf, rect, state, slots, now_ms, false)
}

/// [`render_rail`] with the `done` fade (motion on).
pub fn render_rail_fading(
    buf: &mut Buffer,
    rect: Rect,
    state: &WallState,
    slots: &SlotRegistry,
    now_ms: i64,
    fade_done: bool,
) -> Vec<SlotClickRegion> {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(colors::FAINT));
    let inner = block.inner(rect);
    block.render(rect, buf);
    let (lines, extras) = rail_render(state, slots, now_ms, fade_done);
    Paragraph::new(lines).render(inner, buf);
    for (row, style) in extras.row_styles {
        if row < inner.height {
            buf.set_style(Rect::new(inner.x, inner.y + row, inner.width, 1), style);
        }
    }
    extras
        .clicks
        .into_iter()
        .filter_map(|c| {
            let start = c.cols.start.min(inner.width);
            let end = c.cols.end.min(inner.width);
            (c.row < inner.height && start < end).then(|| SlotClickRegion {
                rect: Rect::new(inner.x + start, inner.y + c.row, end - start, 1),
                target: c.target,
                slot: c.slot,
                session: c.session,
                workspace: c.workspace,
            })
        })
        .collect()
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
        let rows = texts(&rail_lines(store.state(), &SlotRegistry::core(), 60_000));
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
        let rows = texts(&rail_lines(store.state(), &SlotRegistry::core(), 0));
        assert_eq!(rows[0], " 1 a  ● 2");
    }

    #[test]
    fn needs_input_rows_are_amber_marker_never_is() {
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![ws("a")]);
        store.sessions_fetched(vec![si("a", "blocked", "needs-input", Some(0))]);
        let line = &rail_lines(store.state(), &SlotRegistry::core(), 60_000)[1];
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
        let lines = rail_lines(store.state(), &SlotRegistry::core(), 0);
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
        let lines = rail_lines(store.state(), &SlotRegistry::core(), 0);
        let rows = texts(&lines);
        assert_eq!(rows, [" elite-traders", " 1 et-a", " 2 et-b", " 3 solo"]);
        assert_eq!(lines[0].spans[0].style.fg, Some(colors::FAINT));
    }

    #[test]
    fn rail_row_and_header_items_paint_and_report_their_own_click_regions() {
        use crate::actions::ids;
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![ws("alpha"), ws("beta")]);
        store.sessions_fetched(vec![si("alpha", "one", "idle", None), si("beta", "two", "idle", None)]);
        let mut slots = SlotRegistry::core();
        slots.register(SlotId::RailRow, |ctx: &SlotCtx| {
            let s = ctx.session.unwrap();
            vec![SlotItem {
                on_click: Some(ClickTarget::new(ids::SESSION_CLOSE).from_plugin("p")),
                ..SlotItem::new(vec![Span::raw(format!(" x{}", s.label.len()))], 1)
            }]
        });
        slots.register(SlotId::RailHeader, |ctx: &SlotCtx| {
            if ctx.group.is_some_and(|g| g.name == "beta") {
                vec![SlotItem {
                    on_click: Some(ClickTarget::new(ids::SESSION_SPAWN).from_plugin("p")),
                    ..SlotItem::new(vec![Span::raw(" [+]")], 1)
                }]
            } else {
                Vec::new()
            }
        });
        let rect = Rect::new(2, 1, 28, 10);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 12));
        let regions = render_rail(&mut buf, rect, store.state(), &slots, 0);
        // rows: 0 alpha, 1 one, 2 [+], 3 beta, 4 two
        let text = |r: &SlotClickRegion| -> String {
            (r.rect.x..r.rect.x + r.rect.width).map(|x| buf[(x, r.rect.y)].symbol()).collect()
        };
        let by_slot = |slot: SlotId| regions.iter().filter(move |r| r.slot == slot).collect::<Vec<_>>();
        let rows = by_slot(SlotId::RailRow);
        assert_eq!(rows.len(), 2);
        assert_eq!(text(rows[0]), " x3");
        assert_eq!(rows[0].rect.y, 1 + 1, "alpha's session row");
        assert_eq!(rows[0].session.as_deref(), Some("garage/alpha/one"));
        assert_eq!(rows[1].rect.y, 1 + 4);
        let header = by_slot(SlotId::RailHeader);
        assert_eq!(header.len(), 1);
        assert_eq!(text(header[0]), " [+]");
        assert_eq!(header[0].rect.y, 1 + 2);
        assert_eq!(header[0].workspace.as_deref(), Some("beta"));
        assert_eq!(header[0].session, None);
        // The painted rows are exactly what rail_lines says, plus the item.
        let lines = texts(&rail_lines(store.state(), &slots, 0));
        assert_eq!(lines[1], " ▸ ○ one x3", "focused session carries the marker");
    }

    #[test]
    fn extra_rows_paint_with_their_style_and_click_across_the_whole_row() {
        use crate::actions::ids;
        use ratatui::style::Color;
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![ws("alpha")]);
        store.sessions_fetched(vec![si("alpha", "one", "idle", None)]);
        let mut slots = SlotRegistry::core();
        slots.register(SlotId::RailExtraRows, |_: &SlotCtx| {
            vec![SlotItem {
                on_click: Some(
                    ClickTarget::new(ids::SESSION_RESTORE).from_plugin("closed").about_session("garage/alpha/gone"),
                ),
                row_style: Some(Style::default().bg(Color::Indexed(236))),
                ..SlotItem::new(vec![Span::raw(" ↺ gone")], 1)
            }]
        });
        let rect = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(rect);
        let regions = render_rail(&mut buf, rect, store.state(), &slots, 0);
        // rows: 0 alpha, 1 one, 2 extra
        assert_eq!(regions.len(), 1);
        let r = &regions[0];
        assert_eq!((r.rect.y, r.rect.x, r.rect.width), (2, 0, 19), "the whole inner row");
        assert_eq!(r.session.as_deref(), Some("garage/alpha/gone"), "the item's own session, not a row's");
        assert_eq!(r.workspace.as_deref(), Some("alpha"));
        assert_eq!(r.slot, SlotId::RailExtraRows);
        assert_eq!(buf[(1, 2)].symbol(), "↺");
        assert_eq!(buf[(15, 2)].bg, Color::Indexed(236), "row style reaches past the text");
        assert_eq!(buf[(1, 1)].bg, Color::Reset, "other rows untouched");
    }

    #[test]
    fn regions_are_clipped_to_the_rails_inner_width() {
        use crate::actions::ids;
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![ws("alpha")]);
        store.sessions_fetched(vec![si("alpha", "one", "idle", None)]);
        let mut slots = SlotRegistry::core();
        slots.register(SlotId::RailRow, |_: &SlotCtx| {
            vec![SlotItem {
                on_click: Some(ClickTarget::new(ids::SESSION_CLOSE).from_plugin("p")),
                ..SlotItem::new(vec![Span::raw(" a-very-long-button")], 1)
            }]
        });
        let rect = Rect::new(0, 0, 14, 4); // inner width 13 (right border)
        let mut buf = Buffer::empty(rect);
        let regions = render_rail(&mut buf, rect, store.state(), &slots, 0);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].rect.right(), 13);
    }

    #[test]
    fn empty_rail_names_the_state() {
        let store = WallStore::new();
        let rows = texts(&rail_lines(store.state(), &SlotRegistry::core(), 0));
        assert_eq!(rows, [" no workspaces"]);
    }
}
