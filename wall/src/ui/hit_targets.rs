//! Pure click hit-mapping (port of `tui/lib/ui/hit_targets.dart`):
//! region-local cell coordinates → semantic target (tile index, rail row,
//! triage queue row). Everything here is pure so it unit-tests without a
//! terminal — the click regions in the render layer are thin adapters over
//! these.

use crate::state::salience::WorkspaceGroup;
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
/// one line per row: each group's header followed by its sessions) to its
/// target. `None` for rows past the last group (or an empty rail).
pub fn rail_target_at(groups: &[WorkspaceGroup], row: i32) -> Option<RailTarget> {
    if row < 0 {
        return None;
    }
    let row = row as usize;
    let mut line = 0usize;
    for (g, group) in groups.iter().enumerate() {
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
}
