//! Pure grid math (port of `tui/lib/ui/grid_layout.dart` — spec tui-wall:
//! "Grid layout with a six-tile cap": `ceil(sqrt(n))` columns, row-major).
//! The cap and LRU swap-in live in WallState/WallStore; this file only
//! answers "where does slot i go".

/// Row-major position of a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSlot {
    pub row: usize,
    pub col: usize,
}

/// Integer cell rect for a slot (grid-local cells).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Number of columns for `n` tiles: `ceil(sqrt(n))`. 0 for an empty grid.
pub fn grid_columns(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let mut cols = (n as f64).sqrt() as usize;
    while cols * cols < n {
        cols += 1;
    }
    cols
}

/// Number of rows for `n` tiles at [`grid_columns`] columns.
pub fn grid_row_count(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let cols = grid_columns(n);
    n.div_ceil(cols)
}

/// Row-major position of slot `index` in an `n`-tile grid.
pub fn grid_slot(index: usize, n: usize) -> GridSlot {
    debug_assert!(index < n);
    let cols = grid_columns(n);
    GridSlot {
        row: index / cols,
        col: index % cols,
    }
}

/// Slot indices per row, row-major — the shape the render loop walks.
/// The last row may be short; the renderer pads it so columns stay aligned.
pub fn grid_rows(n: usize) -> Vec<Vec<usize>> {
    let cols = grid_columns(n);
    if cols == 0 {
        return Vec::new();
    }
    (0..n)
        .step_by(cols)
        .map(|start| (start..(start + cols).min(n)).collect())
        .collect()
}

/// Integer cell rect for slot `index` of an `n`-tile grid filling a
/// `width`×`height` area. Edges land on `k*extent/count` boundaries so the
/// rects tile the area exactly (no gaps, no overlap, remainders spread).
///
/// The grid renders as absolutely-positioned tiles — NOT nested layouts — so
/// each tile keeps its terminal buffer across reshapes; only its rect
/// changes.
pub fn grid_cell_rect(index: usize, n: usize, width: i32, height: i32) -> GridRect {
    let cols = grid_columns(n) as i32;
    let rows = grid_row_count(n) as i32;
    let slot = grid_slot(index, n);
    let (col, row) = (slot.col as i32, slot.row as i32);
    let x0 = (col * width) / cols;
    let x1 = ((col + 1) * width) / cols;
    let y0 = (row * height) / rows;
    let y1 = ((row + 1) * height) / rows;
    GridRect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

#[cfg(test)]
mod tests {
    //! Port of `tui/test/grid_layout_test.dart`.
    use super::*;

    #[test]
    fn grid_columns_is_ceil_sqrt_n_for_every_wall_size() {
        assert_eq!(grid_columns(0), 0);
        assert_eq!(grid_columns(1), 1);
        assert_eq!(grid_columns(2), 2);
        assert_eq!(grid_columns(3), 2);
        assert_eq!(grid_columns(4), 2);
        assert_eq!(grid_columns(5), 3);
        assert_eq!(grid_columns(6), 3);
    }

    #[test]
    fn rows_cover_all_tiles_at_the_column_count() {
        assert_eq!(grid_row_count(0), 0);
        assert_eq!(grid_row_count(1), 1);
        assert_eq!(grid_row_count(2), 1); // 2 cols × 1 row
        assert_eq!(grid_row_count(3), 2); // 2 cols → 2+1
        assert_eq!(grid_row_count(4), 2); // 2×2
        assert_eq!(grid_row_count(5), 2); // 3 cols → 3+2
        assert_eq!(grid_row_count(6), 2); // 3×2
    }

    #[test]
    fn row_major_placement() {
        // 5 tiles → 3 columns: [0 1 2] / [3 4]
        assert_eq!(grid_slot(0, 5), GridSlot { row: 0, col: 0 });
        assert_eq!(grid_slot(2, 5), GridSlot { row: 0, col: 2 });
        assert_eq!(grid_slot(3, 5), GridSlot { row: 1, col: 0 });
        assert_eq!(grid_slot(4, 5), GridSlot { row: 1, col: 1 });
        // 4 tiles → 2×2
        assert_eq!(grid_slot(2, 4), GridSlot { row: 1, col: 0 });
        assert_eq!(grid_slot(3, 4), GridSlot { row: 1, col: 1 });
    }

    #[test]
    fn grid_rows_walks_every_slot_exactly_once_row_major() {
        assert!(grid_rows(0).is_empty());
        assert_eq!(grid_rows(1), [vec![0]]);
        assert_eq!(grid_rows(3), [vec![0, 1], vec![2]]);
        assert_eq!(grid_rows(6), [vec![0, 1, 2], vec![3, 4, 5]]);
    }

    #[test]
    fn cell_rects_tile_the_area_exactly_no_gaps_no_overlap() {
        let (width, height) = (173, 51); // deliberately not divisible
        for n in 1..=6 {
            let mut area = 0;
            for i in 0..n {
                let r = grid_cell_rect(i, n, width, height);
                assert!(r.width > 0);
                assert!(r.height > 0);
                assert!(r.x + r.width <= width);
                assert!(r.y + r.height <= height);
                area += r.width * r.height;
            }
            // Full rows tile the width; a short last row leaves blank cells,
            // so total area is at most the full area.
            let cols = grid_columns(n);
            let rows = grid_row_count(n);
            assert!(area <= width * height);
            if n == cols * rows {
                assert_eq!(
                    area,
                    width * height,
                    "a full {cols} x {rows} grid must cover everything"
                );
            }
        }
        // Adjacent cells share edges exactly (n=6: 3x2).
        let a = grid_cell_rect(0, 6, width, height);
        let b = grid_cell_rect(1, 6, width, height);
        let d = grid_cell_rect(3, 6, width, height);
        assert_eq!(b.x, a.x + a.width);
        assert_eq!(d.y, a.y + a.height);
    }

    #[test]
    fn rows_agree_with_grid_slot_for_all_n_up_to_the_cap() {
        for n in 1..=6 {
            let rows = grid_rows(n);
            assert_eq!(rows.len(), grid_row_count(n));
            for (r, row) in rows.iter().enumerate() {
                for (c, slot) in row.iter().enumerate() {
                    assert_eq!(grid_slot(*slot, n), GridSlot { row: r, col: c });
                }
            }
        }
    }
}
