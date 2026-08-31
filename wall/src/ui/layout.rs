//! Wall surface layout (task 4.1/4.5): the rail | grid | strip split, the
//! per-tile rects (including the maximize override), and centered modal
//! rects for the overlays. Pure geometry so it unit-tests without a
//! terminal; the runtime computes one [`WallLayout`] per frame and both the
//! renderer and the mouse router read the SAME rects, so a click can never
//! disagree with the paint.

use ratatui::layout::{Constraint, Layout, Rect};

use crate::ui::grid_layout::grid_cell_rect;

/// The rail's fixed column width (the Dart `Rail.width` default).
pub const RAIL_WIDTH: u16 = 28;

#[derive(Clone, Debug, Default)]
pub struct WallLayout {
    pub rail: Rect,
    /// The tile area: with a group frame, this is already inset one cell
    /// inside [`WallLayout::group_frame`] (the frame's border lives in that
    /// outer rect, never inside `grid`).
    pub grid: Rect,
    pub strip: Rect,
    /// The one-line view strip directly above the grid (spec tui-views
    /// "View strip and group frame"), zero-height when fewer than 2 views —
    /// callers key rendering off the height, not a separate bool.
    pub view_strip: Rect,
    /// The group-frame border rect — one cell larger than `grid` on every
    /// side — when the focused view has 2+ sessions; `None` for a solo view
    /// (frameless, spec: "the frame's absence marks individual").
    pub group_frame: Option<Rect>,
    /// One rect per gridded session (same order as
    /// `WallState::gridded_session_ids`). With a maximized tile, its rect is
    /// the full grid area while siblings KEEP their normal grid-cell rects —
    /// they stay live (and PTY-sized) underneath, they just aren't drawn and
    /// receive no input.
    pub tiles: Vec<Rect>,
    /// Index into `tiles` of the maximized tile, if any.
    pub maximized: Option<usize>,
}

/// Split the frame and place every tile. `maximized` must already be
/// validated as an index into the gridded set. `view_strip` reserves a
/// one-line row above the grid (spec tui-views: only when the focused
/// workspace has 2+ views); `framed` insets the grid by one cell on every
/// side for the group-frame border (only when the focused view has 2+
/// sessions) — both are the caller's (runtime.rs) precomputed booleans, so
/// this stays pure geometry with no state dependency.
pub fn wall_layout(
    area: Rect,
    n_tiles: usize,
    maximized: Option<usize>,
    view_strip: bool,
    framed: bool,
) -> WallLayout {
    let [main, strip] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let [rail, grid_full] =
        Layout::horizontal([Constraint::Length(RAIL_WIDTH.min(main.width)), Constraint::Fill(1)])
            .areas(main);

    let (view_strip_rect, grid_area) = if view_strip && grid_full.height > 0 {
        let [vs, rest] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(grid_full);
        (vs, rest)
    } else {
        (
            Rect { x: grid_full.x, y: grid_full.y, width: grid_full.width, height: 0 },
            grid_full,
        )
    };

    let (group_frame, grid) = if framed && grid_area.width >= 2 && grid_area.height >= 2 {
        (
            Some(grid_area),
            Rect {
                x: grid_area.x + 1,
                y: grid_area.y + 1,
                width: grid_area.width - 2,
                height: grid_area.height - 2,
            },
        )
    } else {
        (None, grid_area)
    };

    let tiles = (0..n_tiles)
        .map(|i| {
            if maximized == Some(i) {
                return grid;
            }
            let r = grid_cell_rect(i, n_tiles, i32::from(grid.width), i32::from(grid.height));
            Rect {
                x: grid.x + r.x as u16,
                y: grid.y + r.y as u16,
                width: r.width as u16,
                height: r.height as u16,
            }
        })
        .collect();

    WallLayout {
        rail,
        grid,
        strip,
        view_strip: view_strip_rect,
        group_frame,
        tiles,
        maximized: maximized.filter(|&i| i < n_tiles),
    }
}

/// A tile's inner (terminal) area: the rect minus its 1-cell border. This is
/// the PTY size pushed via TIOCSWINSZ. Clamped to at least 2×2 like the
/// spike so degenerate rects never produce a zero-size PTY.
pub fn tile_inner(rect: Rect) -> (u16, u16) {
    (
        rect.width.saturating_sub(2).max(2),
        rect.height.saturating_sub(2).max(2),
    )
}

/// Center a `width`×`height` box in `area`, clamped to fit.
pub fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// The triage/help/add-workspace modal width rule from the Dart overlays:
/// full width minus 8, clamped to `[min, max]`.
pub fn modal_width(area_width: u16, min: u16, max: u16) -> u16 {
    (area_width.saturating_sub(8)).clamp(min, max).min(area_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn splits_rail_grid_strip() {
        let l = wall_layout(area(120, 40), 0, None, false, false);
        assert_eq!(l.rail, Rect::new(0, 0, 28, 39));
        assert_eq!(l.grid, Rect::new(28, 0, 92, 39));
        assert_eq!(l.strip, Rect::new(0, 39, 120, 1));
        assert_eq!(l.view_strip.height, 0, "no view strip by default");
        assert_eq!(l.group_frame, None);
    }

    #[test]
    fn tiles_tile_the_grid_area_exactly_for_a_full_grid() {
        let l = wall_layout(area(120, 40), 6, None, false, false);
        assert_eq!(l.tiles.len(), 6);
        let total: u32 = l
            .tiles
            .iter()
            .map(|r| u32::from(r.width) * u32::from(r.height))
            .sum();
        assert_eq!(total, u32::from(l.grid.width) * u32::from(l.grid.height));
        for r in &l.tiles {
            assert!(r.x >= l.grid.x && r.x + r.width <= l.grid.x + l.grid.width);
            assert!(r.y >= l.grid.y && r.y + r.height <= l.grid.y + l.grid.height);
        }
    }

    #[test]
    fn maximized_tile_takes_the_full_grid_siblings_keep_their_cells() {
        let normal = wall_layout(area(120, 40), 4, None, false, false);
        let l = wall_layout(area(120, 40), 4, Some(2), false, false);
        assert_eq!(l.maximized, Some(2));
        assert_eq!(l.tiles[2], l.grid, "maximized rect = full grid area");
        for i in [0, 1, 3] {
            assert_eq!(l.tiles[i], normal.tiles[i], "sibling {i} keeps its cell");
        }
    }

    #[test]
    fn view_strip_reserves_one_row_above_the_grid() {
        let l = wall_layout(area(120, 40), 2, None, true, false);
        assert_eq!(l.view_strip, Rect::new(28, 0, 92, 1));
        assert_eq!(l.grid, Rect::new(28, 1, 92, 38), "grid shifts down by one row");
        assert_eq!(l.strip, Rect::new(0, 39, 120, 1), "bottom strip untouched");
    }

    #[test]
    fn no_view_strip_reserves_no_row() {
        let with = wall_layout(area(120, 40), 2, None, true, false);
        let without = wall_layout(area(120, 40), 2, None, false, false);
        assert_eq!(without.view_strip.height, 0);
        assert_eq!(without.grid.height, with.grid.height + 1);
    }

    #[test]
    fn group_frame_insets_the_grid_by_one_cell_on_every_side() {
        let unframed = wall_layout(area(120, 40), 2, None, false, false);
        let l = wall_layout(area(120, 40), 2, None, false, true);
        let frame = l.group_frame.expect("framed");
        assert_eq!(frame, unframed.grid, "the frame border sits where the grid used to start");
        assert_eq!(
            l.grid,
            Rect::new(frame.x + 1, frame.y + 1, frame.width - 2, frame.height - 2)
        );
    }

    #[test]
    fn view_strip_and_group_frame_compose() {
        let l = wall_layout(area(120, 40), 2, None, true, true);
        assert_eq!(l.view_strip, Rect::new(28, 0, 92, 1));
        let frame = l.group_frame.expect("framed");
        assert_eq!(frame, Rect::new(28, 1, 92, 38), "frame sits below the view strip");
        assert_eq!(l.grid, Rect::new(29, 2, 90, 36));
    }

    #[test]
    fn a_tiny_area_never_produces_a_degenerate_frame_inset() {
        // Too small for a 1-cell border on every side: no frame, no panic.
        let l = wall_layout(Rect::new(0, 0, 30, 2), 1, None, false, true);
        assert_eq!(l.group_frame, None);
    }

    #[test]
    fn tile_inner_subtracts_the_border_with_a_floor() {
        assert_eq!(tile_inner(Rect::new(0, 0, 80, 24)), (78, 22));
        assert_eq!(tile_inner(Rect::new(0, 0, 3, 2)), (2, 2), "floor at 2x2");
    }

    #[test]
    fn centered_rect_centers_and_clamps() {
        assert_eq!(centered_rect(area(100, 40), 60, 10), Rect::new(20, 15, 60, 10));
        assert_eq!(centered_rect(area(10, 4), 60, 10), Rect::new(0, 0, 10, 4));
    }

    #[test]
    fn modal_width_follows_the_dart_clamp() {
        assert_eq!(modal_width(100, 24, 78), 78);
        assert_eq!(modal_width(60, 24, 78), 52);
        assert_eq!(modal_width(20, 24, 78), 20, "never wider than the area");
    }

    #[test]
    fn narrow_terminal_never_underflows() {
        let l = wall_layout(area(20, 5), 2, None, false, false);
        assert_eq!(l.rail.width, 20, "rail clamped to the area");
        assert_eq!(l.grid.width, 0);
        assert_eq!(l.tiles.len(), 2);
    }
}
