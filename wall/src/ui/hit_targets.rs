//! Pure click hit-mapping (port of `tui/lib/ui/hit_targets.dart`):
//! region-local cell coordinates → semantic target (tile index, rail row,
//! triage queue row). Everything here is pure so it unit-tests without a
//! terminal — the click regions in the render layer are thin adapters over
//! these.

use std::ops::Range;

use ratatui::layout::Rect;

use crate::actions::{ids, ActionCtx, ActionId, ActionSource};
use crate::state::salience::WorkspaceGroup;
use crate::state::wall_state::{KeyLayer, OverlayKind, WallState};
use crate::ui::layout::WallLayout;
use crate::ui::triage::triage_queue_rows;
use crate::ui::grid_layout::grid_cell_rect;

/// Tile slot index containing the grid-local point (`col`,`row`), or `None`
/// when the point falls in the dead space after the last short row. Walks
/// the same [`grid_cell_rect`] rects the renderer positions tiles with, so a
/// click can never disagree with the paint.
pub fn tile_index_at(col: i32, row: i32, n: usize, width: i32, height: i32) -> Option<usize> {
    if n == 0 || width <= 0 || height <= 0 {
        return None;
    }
    if col < 0 || row < 0 || col >= width || row >= height {
        return None;
    }
    (0..n).find(|&i| {
        let r = grid_cell_rect(i, n, width, height);
        col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
    })
}

/// A rail row resolved from a click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RailTarget {
    /// A workspace header row — the index is the position in the
    /// salience-ordered groups list (the same index the `1`–`9` bindings
    /// use).
    Workspace(usize),
    /// A session row.
    Session(String),
}

/// Maps a rail-content row (0-based, local to the rail — the rail renders
/// one line per row: each group's optional family label, its header, then
/// its sessions) to its target. `None` for a label row, rows past the last
/// group, or an empty rail.
pub fn rail_target_at(groups: &[WorkspaceGroup], row: i32) -> Option<RailTarget> {
    if row < 0 {
        return None;
    }
    let row = row as usize;
    let mut line = 0usize;
    for (g, group) in groups.iter().enumerate() {
        if group.family_label.is_some() {
            if row == line {
                return None;
            }
            line += 1;
        }
        if row == line {
            return Some(RailTarget::Workspace(g));
        }
        line += 1;
        let sessions = &group.sessions;
        if row < line + sessions.len() {
            return Some(RailTarget::Session(sessions[row - line].id.clone()));
        }
        line += sessions.len();
    }
    None
}

/// Rows the triage modal's click region skips before the first queue row:
/// the box border (1) plus its vertical padding (1). The region wraps the
/// whole modal box so a border/padding click is still absorbed (not treated
/// as outside-the-modal), hence the offset.
pub const TRIAGE_MODAL_ROW_OFFSET: i32 = 2;

/// Maps a modal-local row to a queue row index, or `None` for the border,
/// padding, blank line, or footer. `row_count` is the number of queue rows.
pub fn triage_row_index_at(local_row: i32, row_count: usize) -> Option<usize> {
    let i = local_row - TRIAGE_MODAL_ROW_OFFSET;
    if i < 0 || i as usize >= row_count {
        return None;
    }
    Some(i as usize)
}

/// Same border+padding offset as the triage modal (spec tui-views "Move to a
/// group" — the picker uses the same modal shape as `triage`/`workspace_add`).
pub const VIEW_PICKER_MODAL_ROW_OFFSET: i32 = 2;

/// Maps a picker-modal-local row to an entry index (every view name plus the
/// trailing "new group…" row), or `None` for the border, padding, blank
/// line, or footer.
pub fn view_picker_row_index_at(local_row: i32, entry_count: usize) -> Option<usize> {
    let i = local_row - VIEW_PICKER_MODAL_ROW_OFFSET;
    if i < 0 || i as usize >= entry_count {
        return None;
    }
    Some(i as usize)
}

/// The rects and columns the last frame painted, as the click resolver
/// needs them (the runtime fills this from its per-frame bookkeeping).
pub struct ClickMap<'a> {
    pub layout: &'a WallLayout,
    pub view_strip_targets: &'a [(Range<u16>, String)],
    pub triage_rect: Option<Rect>,
    pub ws_add_rect: Option<Rect>,
    pub view_picker_rect: Option<Rect>,
    /// Entries in the view picker (every other view plus "new group…").
    pub view_picker_entries: usize,
    /// The picker is in its "new group…" text-input sub-mode.
    pub view_picker_typing: bool,
    pub badge_cols: Option<&'a Range<u16>>,
    pub pet_cols: Option<&'a Range<u16>>,
}

/// The session id and rect under a grid point, honoring maximize (a
/// maximized tile covers the whole grid, so every grid point targets it).
pub fn tile_at(layout: &WallLayout, state: &WallState, col: u16, row: u16) -> Option<(String, Rect)> {
    if !layout.grid.contains((col, row).into()) {
        return None;
    }
    let ids = &state.gridded_session_ids;
    let index = match layout.maximized {
        Some(i) => i,
        None => tile_index_at(
            i32::from(col - layout.grid.x),
            i32::from(row - layout.grid.y),
            ids.len(),
            i32::from(layout.grid.width),
            i32::from(layout.grid.height),
        )?,
    };
    Some((ids.get(index)?.clone(), *layout.tiles.get(index)?))
}

/// Resolve a left click to the action it triggers, with its context — the
/// same ids the keymap resolves keys to (a rail header is `workspace.focus`
/// like `1`–`9`, a triage row is `jump.blocked` like Enter in the queue, the
/// strip badge is `overlay.open.queue` like `A`). `None` for inert points:
/// family label rows, borders, padding, an engaged tile's own area, and
/// anything under a modal overlay. The tile-text link click is not an
/// action and is handled by the runtime ahead of this.
pub fn action_at(
    map: &ClickMap,
    state: &WallState,
    col: u16,
    row: u16,
) -> Option<(ActionId, ActionCtx)> {
    let ctx = || ActionCtx::from_focus(state, ActionSource::Mouse { col, row });
    let point = (col, row).into();
    let layout = map.layout;
    // Overlay layer: the whole screen is the overlay's (a barrier — clicks
    // never reach the wall underneath).
    if let Some(kind) = state.overlay {
        return match kind {
            // Any click dismisses help.
            OverlayKind::Help => Some((ids::OVERLAY_CLOSE, ctx())),
            OverlayKind::TriageQueue => {
                let rect = map.triage_rect.filter(|r| r.contains(point));
                let Some(rect) = rect else {
                    return Some((ids::OVERLAY_CLOSE, ctx())); // outside-modal click = Esc
                };
                let rows = triage_queue_rows(&state.sessions);
                let i = triage_row_index_at(i32::from(row - rect.y), rows.len())?;
                // A queue row click selects and jump-engages (Enter's landing).
                Some((
                    ids::JUMP_BLOCKED,
                    ctx().with_session(rows[i].id.clone()).with_selection(i),
                ))
            }
            OverlayKind::WorkspaceAdd => {
                let inside = map.ws_add_rect.is_some_and(|r| r.contains(point));
                (!inside).then(|| (ids::OVERLAY_CLOSE, ctx()))
            }
            OverlayKind::ViewPicker => {
                let rect = map.view_picker_rect?;
                if !rect.contains(point) {
                    return Some((ids::OVERLAY_CLOSE, ctx()));
                }
                if map.view_picker_typing {
                    return None;
                }
                let i = view_picker_row_index_at(i32::from(row - rect.y), map.view_picker_entries)?;
                Some((ids::OVERLAY_VIEW_PICKER_PICK, ctx().with_selection(i)))
            }
            // Modal: clicks anywhere are swallowed — an outside click must
            // not quietly "keep" a worktree the user never saw.
            OverlayKind::WorktreeFinish => None,
        };
    }
    // View-strip click: focus that view.
    if layout.view_strip.height > 0 && row == layout.view_strip.y {
        let name = map
            .view_strip_targets
            .iter()
            .find(|(r, _)| r.contains(&col))
            .map(|(_, name)| name.clone())?;
        return Some((ids::VIEW_FOCUS, ctx().with_view(name)));
    }
    // Grid click: focus AND engage (same landing as focus+Enter); a click on
    // a restorable placeholder restores it; clicking the engaged tile itself
    // does nothing extra.
    if let Some((id, _)) = tile_at(layout, state, col, row) {
        let live = state.session_by_id(Some(&id)).is_some_and(|s| s.live());
        if !live {
            return Some((ids::SESSION_RESTORE, ctx().with_session(id)));
        }
        if state.layer == KeyLayer::Engaged && state.focused_session_id.as_deref() == Some(id.as_str()) {
            return None;
        }
        return Some((ids::SESSION_ENGAGE, ctx().with_session(id)));
    }
    // Rail click: header focuses the workspace, session row focuses the
    // session — never engages.
    if layout.rail.contains(point) {
        return match rail_target_at(&state.groups, i32::from(row - layout.rail.y))? {
            RailTarget::Workspace(index) => {
                Some((ids::WORKSPACE_FOCUS, ctx().with_index(Some(index))))
            }
            RailTarget::Session(id) => Some((ids::SESSION_FOCUS, ctx().with_session(id))),
        };
    }
    // Pet click first, so an alert pet sitting near the badge never gets
    // swallowed by it; then the strip badge (the `A` binding).
    if row >= layout.strip.y && map.pet_cols.is_some_and(|r| r.contains(&col)) {
        return Some((ids::PET_PET, ctx()));
    }
    if row >= layout.strip.y && map.badge_cols.is_some_and(|r| r.contains(&col)) {
        return Some((ids::OVERLAY_OPEN_QUEUE, ctx()));
    }
    None
}

#[cfg(test)]
mod tests {
    //! Port of `tui/test/hit_targets_test.dart` — point→tile index via
    //! gridCellRect, point→rail row, triage modal rows.
    use super::*;
    use crate::state::wall_state::WallSession;

    fn session(id: &str) -> WallSession {
        WallSession {
            id: id.to_owned(),
            workspace: "ws".to_owned(),
            label: id.to_owned(),
            dir: None,
            status: "idle".to_owned(),
            since: None,
            message: None,
            branch: None,
            worktree: false,
            title: None,
            context: None,
        }
    }

    fn rail_group(name: &str, sessions: Vec<WallSession>) -> WorkspaceGroup {
        WorkspaceGroup {
            name: name.to_owned(),
            dir: None,
            branch: None,
            registered: true,
            sessions,
            family_label: None,
        }
    }

    // ── tileIndexAt ─────────────────────────────────────────────────────

    #[test]
    fn single_tile_every_in_bounds_point_maps_to_slot_0() {
        assert_eq!(tile_index_at(0, 0, 1, 80, 24), Some(0));
        assert_eq!(tile_index_at(79, 23, 1, 80, 24), Some(0));
        assert_eq!(tile_index_at(40, 12, 1, 80, 24), Some(0));
    }

    #[test]
    fn out_of_bounds_and_degenerate_inputs_map_to_none() {
        assert_eq!(tile_index_at(-1, 0, 1, 80, 24), None);
        assert_eq!(tile_index_at(0, -1, 1, 80, 24), None);
        assert_eq!(tile_index_at(80, 0, 1, 80, 24), None);
        assert_eq!(tile_index_at(0, 24, 1, 80, 24), None);
        assert_eq!(tile_index_at(0, 0, 0, 80, 24), None);
        assert_eq!(tile_index_at(0, 0, 3, 0, 24), None);
    }

    #[test]
    fn agrees_with_grid_cell_rect_for_every_cell_of_every_point() {
        // The invariant that matters: a click at (col,row) resolves to the
        // slot whose painted rect contains it — for every point of a small
        // area and every grid population the wall can show.
        let (width, height) = (17, 11); // odd sizes exercise remainder spread
        for n in 1..=6usize {
            for row in 0..height {
                for col in 0..width {
                    let hit = tile_index_at(col, row, n, width, height);
                    let expected = (0..n).find(|&i| {
                        let r = grid_cell_rect(i, n, width, height);
                        col >= r.x
                            && col < r.x + r.width
                            && row >= r.y
                            && row < r.y + r.height
                    });
                    assert_eq!(hit, expected, "n={n} point=({col},{row})");
                }
            }
        }
    }

    #[test]
    fn four_tiles_in_8x4_quadrant_corners_land_on_the_right_slots() {
        // cols=2, rows=2; cells are 4x2.
        assert_eq!(tile_index_at(0, 0, 4, 8, 4), Some(0));
        assert_eq!(tile_index_at(3, 1, 4, 8, 4), Some(0));
        assert_eq!(tile_index_at(4, 0, 4, 8, 4), Some(1));
        assert_eq!(tile_index_at(0, 2, 4, 8, 4), Some(2));
        assert_eq!(tile_index_at(7, 3, 4, 8, 4), Some(3));
    }

    #[test]
    fn short_last_row_dead_space_maps_to_none_not_a_phantom_tile() {
        // n=3 → cols=2, rows=2; slot 2 is alone on the bottom row.
        let (width, height) = (8, 4);
        assert_eq!(tile_index_at(1, 3, 3, width, height), Some(2));
        // Bottom-right quadrant has no slot.
        assert_eq!(tile_index_at(7, 3, 3, width, height), None);
    }

    // ── railTargetAt ────────────────────────────────────────────────────

    fn groups() -> Vec<WorkspaceGroup> {
        vec![
            rail_group("alpha", vec![session("a1"), session("a2")]),
            rail_group("beta", vec![session("b1")]),
        ]
    }

    #[test]
    fn header_rows_map_to_workspace_targets_with_group_index() {
        assert_eq!(rail_target_at(&groups(), 0), Some(RailTarget::Workspace(0)));
        assert_eq!(rail_target_at(&groups(), 3), Some(RailTarget::Workspace(1)));
    }

    #[test]
    fn session_rows_map_to_session_ids() {
        assert_eq!(
            rail_target_at(&groups(), 1),
            Some(RailTarget::Session("a1".to_owned()))
        );
        assert_eq!(
            rail_target_at(&groups(), 2),
            Some(RailTarget::Session("a2".to_owned()))
        );
        assert_eq!(
            rail_target_at(&groups(), 4),
            Some(RailTarget::Session("b1".to_owned()))
        );
    }

    #[test]
    fn rows_past_the_last_group_and_negatives_map_to_none() {
        assert_eq!(rail_target_at(&groups(), 5), None);
        assert_eq!(rail_target_at(&groups(), 99), None);
        assert_eq!(rail_target_at(&groups(), -1), None);
        assert_eq!(rail_target_at(&[], 0), None);
    }

    #[test]
    fn a_session_less_group_is_a_single_header_row() {
        let gs = vec![
            rail_group("empty", vec![]),
            rail_group("full", vec![session("x")]),
        ];
        assert_eq!(rail_target_at(&gs, 0), Some(RailTarget::Workspace(0)));
        assert_eq!(rail_target_at(&gs, 1), Some(RailTarget::Workspace(1)));
        assert_eq!(
            rail_target_at(&gs, 2),
            Some(RailTarget::Session("x".to_owned()))
        );
    }

    #[test]
    fn a_family_label_row_maps_to_none_and_shifts_rows_below_it() {
        let mut first = rail_group("alpha", vec![session("a1")]);
        first.family_label = Some("fam".to_owned());
        let gs = vec![first, rail_group("beta", vec![session("b1")])];
        // rows: 0 label, 1 alpha, 2 a1, 3 beta, 4 b1
        assert_eq!(rail_target_at(&gs, 0), None);
        assert_eq!(rail_target_at(&gs, 1), Some(RailTarget::Workspace(0)));
        assert_eq!(
            rail_target_at(&gs, 2),
            Some(RailTarget::Session("a1".to_owned()))
        );
        assert_eq!(rail_target_at(&gs, 3), Some(RailTarget::Workspace(1)));
        assert_eq!(
            rail_target_at(&gs, 4),
            Some(RailTarget::Session("b1".to_owned()))
        );
        assert_eq!(rail_target_at(&gs, 5), None);
    }

    // ── triageRowIndexAt ────────────────────────────────────────────────

    #[test]
    fn rows_sit_below_the_border_plus_padding_offset() {
        assert_eq!(TRIAGE_MODAL_ROW_OFFSET, 2);
        assert_eq!(triage_row_index_at(2, 3), Some(0));
        assert_eq!(triage_row_index_at(4, 3), Some(2));
    }

    #[test]
    fn border_padding_blank_line_and_footer_map_to_none() {
        assert_eq!(triage_row_index_at(0, 3), None); // top border
        assert_eq!(triage_row_index_at(1, 3), None); // vertical padding
        assert_eq!(triage_row_index_at(5, 3), None); // blank line after rows
        assert_eq!(triage_row_index_at(6, 3), None); // footer
        assert_eq!(triage_row_index_at(2, 0), None); // empty queue
        assert_eq!(triage_row_index_at(-1, 3), None);
    }

    // ── viewPickerRowIndexAt ────────────────────────────────────────────

    #[test]
    fn picker_rows_sit_below_the_border_plus_padding_offset() {
        assert_eq!(VIEW_PICKER_MODAL_ROW_OFFSET, 2);
        assert_eq!(view_picker_row_index_at(2, 3), Some(0));
        assert_eq!(view_picker_row_index_at(4, 3), Some(2), "the new group… row");
    }

    #[test]
    fn picker_border_padding_and_footer_map_to_none() {
        assert_eq!(view_picker_row_index_at(0, 3), None);
        assert_eq!(view_picker_row_index_at(1, 3), None);
        assert_eq!(view_picker_row_index_at(5, 3), None);
        assert_eq!(view_picker_row_index_at(-1, 3), None);
    }

    // ── action_at: clicks resolve to the same ids as keys ───────────────

    mod actions {
        use super::super::*;
        use crate::api::models::{SessionInfo, WorkspaceInfo};
        use crate::keymap::{Keymap, Layer};
        use crate::state::store::WallStore;
        use crate::ui::layout::wall_layout;

        fn ws(name: &str, dir: &str) -> WorkspaceInfo {
            WorkspaceInfo { name: name.to_owned(), dir: Some(dir.to_owned()), branch: None }
        }
        fn si(workspace: &str, label: &str, status: &str) -> SessionInfo {
            SessionInfo {
                id: format!("garage/{workspace}/{label}"),
                workspace: workspace.to_owned(),
                label: label.to_owned(),
                dir: None,
                attached: false,
                status: status.to_owned(),
                since: Some(0),
                message: None,
                branch: None,
                restorable: status == "restorable",
                title: None,
                context: None,
            }
        }

        fn store() -> WallStore {
            let mut store = WallStore::new();
            store.workspaces_fetched(vec![
                ws("et-a", "/w/fam/et-a"),
                ws("et-b", "/w/fam/et-b"),
            ]);
            store.sessions_fetched(vec![
                si("et-a", "one", "working"),
                si("et-a", "gone", "restorable"),
                si("et-b", "stuck", "needs-input"),
                si("et-a", "calm", "idle"),
            ]);
            store
        }

        struct Fixture {
            layout: WallLayout,
            targets: Vec<(Range<u16>, String)>,
            badge: Range<u16>,
            pet: Range<u16>,
        }

        fn fixture(store: &WallStore) -> Fixture {
            let n = store.state().gridded_session_ids.len();
            Fixture {
                layout: wall_layout(Rect::new(0, 0, 120, 40), n, None, false, false),
                targets: Vec::new(),
                badge: 80..95,
                pet: 50..60,
            }
        }

        fn click(store: &WallStore, f: &Fixture, col: u16, row: u16) -> Option<(&'static str, ActionCtx)> {
            let map = ClickMap {
                layout: &f.layout,
                view_strip_targets: &f.targets,
                triage_rect: Some(Rect::new(20, 5, 60, 12)),
                ws_add_rect: Some(Rect::new(20, 5, 60, 12)),
                view_picker_rect: Some(Rect::new(20, 5, 60, 12)),
                view_picker_entries: 3,
                view_picker_typing: false,
                badge_cols: Some(&f.badge),
                pet_cols: Some(&f.pet),
            };
            action_at(&map, store.state(), col, row).map(|(id, ctx)| (id.as_str(), ctx))
        }

        fn key_id(layer: Layer, key: &str) -> &'static str {
            Keymap::defaults().resolve(layer, key).unwrap().action.as_str()
        }

        #[test]
        fn rail_header_is_workspace_focus_like_the_number_keys() {
            let store = store();
            let f = fixture(&store);
            // Row 0 is the family label (inert); row 1 the first header.
            assert!(click(&store, &f, 2, f.layout.rail.y).is_none(), "label row is inert");
            let (id, ctx) = click(&store, &f, 2, f.layout.rail.y + 1).unwrap();
            assert_eq!(id, key_id(Layer::Garage, "1"));
            assert_eq!(ctx.index, Some(0));
            assert!(matches!(ctx.source, ActionSource::Mouse { .. }));
        }

        #[test]
        fn rail_session_row_is_session_focus_with_the_session() {
            let store = store();
            let f = fixture(&store);
            let (id, ctx) = click(&store, &f, 2, f.layout.rail.y + 2).unwrap();
            assert_eq!(id, "session.focus");
            assert!(ctx.session.is_some());
            assert!(click(&store, &f, 2, f.layout.rail.y + 30).is_none(), "past the last row");
        }

        #[test]
        fn live_tile_engages_dead_tile_restores() {
            let store = store();
            let f = fixture(&store);
            for (i, rect) in f.layout.tiles.iter().enumerate() {
                let id = &store.state().gridded_session_ids[i];
                let dead = store.state().session_by_id(Some(id)).is_some_and(|s| !s.live());
                let (action, ctx) = click(&store, &f, rect.x + 2, rect.y + 2).unwrap();
                assert_eq!(ctx.session.as_deref(), Some(id.as_str()));
                assert_eq!(action, if dead { "session.restore" } else { key_id(Layer::Garage, "\n") });
            }
        }

        #[test]
        fn engaged_tile_click_is_inert_but_another_tile_migrates() {
            let mut store = store();
            store.engage();
            let f = fixture(&store);
            let focused = store.state().focused_session_id.clone().unwrap();
            let own = store.state().gridded_session_ids.iter().position(|i| *i == focused).unwrap();
            let r = f.layout.tiles[own];
            assert!(click(&store, &f, r.x + 2, r.y + 2).is_none());
            let other = (0..f.layout.tiles.len())
                .find(|i| {
                    let id = &store.state().gridded_session_ids[*i];
                    *i != own && store.state().session_by_id(Some(id)).is_some_and(|s| s.live())
                })
                .unwrap();
            let r = f.layout.tiles[other];
            assert_eq!(click(&store, &f, r.x + 2, r.y + 2).unwrap().0, "session.engage");
        }

        #[test]
        fn strip_badge_is_the_a_key_queue_and_pet_wins_overlap() {
            let store = store();
            let f = fixture(&store);
            let row = f.layout.strip.y;
            assert_eq!(click(&store, &f, 85, row).unwrap().0, key_id(Layer::Garage, "A"));
            assert_eq!(click(&store, &f, 55, row).unwrap().0, "pet.pet");
            assert!(click(&store, &f, 70, row).is_none());
        }

        #[test]
        fn triage_row_click_is_the_queue_enter_action_with_the_session() {
            let mut store = store();
            store.open_overlay(OverlayKind::TriageQueue);
            let f = fixture(&store);
            let (id, ctx) = click(&store, &f, 30, 5 + 2).unwrap();
            assert_eq!(id, key_id(Layer::Overlay(OverlayKind::TriageQueue), "enter"));
            assert_eq!(ctx.session.as_deref(), Some("garage/et-b/stuck"));
            assert_eq!(ctx.selection, Some(0));
            assert!(click(&store, &f, 30, 5).is_none(), "border is inert");
            assert_eq!(click(&store, &f, 1, 1).unwrap().0, "overlay.close", "outside = Esc");
        }

        #[test]
        fn modal_overlays_swallow_or_close() {
            let mut store = store();
            store.open_overlay(OverlayKind::Help);
            let f = fixture(&store);
            assert_eq!(click(&store, &f, 60, 20).unwrap().0, "overlay.close");

            let mut store = self::store();
            store.open_overlay(OverlayKind::WorkspaceAdd);
            assert!(click(&store, &f, 30, 8).is_none(), "inside the form");
            assert_eq!(click(&store, &f, 1, 1).unwrap().0, "overlay.close");

            let mut store = self::store();
            store.open_overlay(OverlayKind::ViewPicker);
            let (id, ctx) = click(&store, &f, 30, 5 + 3).unwrap();
            assert_eq!((id, ctx.selection), ("overlay.view_picker.pick", Some(1)));
        }
    }
}
