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
    pub grid: Rect,
    pub strip: Rect,
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
/// validated as an index into the gridded set.
pub fn wall_layout(area: Rect, n_tiles: usize, maximized: Option<usize>) -> WallLayout {
    let [main, strip] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let [rail, grid] =
        Layout::horizontal([Constraint::Length(RAIL_WIDTH.min(main.width)), Constraint::Fill(1)])
            .areas(main);

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
        let l = wall_layout(area(120, 40), 0, None);
        assert_eq!(l.rail, Rect::new(0, 0, 28, 39));
        assert_eq!(l.grid, Rect::new(28, 0, 92, 39));
        assert_eq!(l.strip, Rect::new(0, 39, 120, 1));
    }

    #[test]
    fn tiles_tile_the_grid_area_exactly_for_a_full_grid() {
        let l = wall_layout(area(120, 40), 6, None);
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
        let normal = wall_layout(area(120, 40), 4, None);
        let l = wall_layout(area(120, 40), 4, Some(2));
        assert_eq!(l.maximized, Some(2));
        assert_eq!(l.tiles[2], l.grid, "maximized rect = full grid area");
        for i in [0, 1, 3] {
            assert_eq!(l.tiles[i], normal.tiles[i], "sibling {i} keeps its cell");
        }
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
        let l = wall_layout(area(20, 5), 2, None);
        assert_eq!(l.rail.width, 20, "rail clamped to the area");
        assert_eq!(l.grid.width, 0);
        assert_eq!(l.tiles.len(), 2);
    }
}
