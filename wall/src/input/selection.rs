//! Mouse selection inside a tile (p25 mouse selection and copy): the pure
//! half — what is selected and what text it holds. Coordinates are
//! tile-local terminal cells `(col, row)` inside the tile's border, the same
//! frame the PTY's screen uses, so the highlight and the copied text can
//! never disagree.

use ratatui::layout::Rect;

/// A drag in one tile. A selection reads like a terminal's stream
/// selection: from the first cell to the last in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub session: String,
    pub anchor: (u16, u16),
    pub head: (u16, u16),
}

impl Selection {
    /// A plain click leaves anchor == head: nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// `(start, end)` in reading order (row first), both inclusive.
    pub fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        let key = |p: (u16, u16)| (p.1, p.0);
        if key(self.anchor) <= key(self.head) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn contains(&self, col: u16, row: u16) -> bool {
        if self.is_empty() {
            return false;
        }
        let (start, end) = self.ordered();
        (row, col) >= (start.1, start.0) && (row, col) <= (end.1, end.0)
    }
}

/// `point` (absolute) clamped into `inner` and made tile-local.
pub fn local_cell(inner: Rect, col: u16, row: u16) -> (u16, u16) {
    let col = col.clamp(
        inner.x,
        (inner.x + inner.width).saturating_sub(1).max(inner.x),
    );
    let row = row.clamp(
        inner.y,
        (inner.y + inner.height).saturating_sub(1).max(inner.y),
    );
    (col - inner.x, row - inner.y)
}

/// Highlight the selection on `buf`: reverse video over each selected cell
/// of the tile's terminal area `inner` (absolute coordinates).
pub fn paint(buf: &mut ratatui::buffer::Buffer, inner: Rect, sel: &Selection) {
    use ratatui::style::Modifier;
    let (start, end) = sel.ordered();
    for row in start.1..=end.1.min(inner.height.saturating_sub(1)) {
        let from = if row == start.1 { start.0 } else { 0 };
        let to = if row == end.1 {
            end.0
        } else {
            inner.width.saturating_sub(1)
        };
        for col in from..=to.min(inner.width.saturating_sub(1)) {
            if let Some(cell) = buf.cell_mut((inner.x + col, inner.y + row)) {
                cell.modifier.insert(Modifier::REVERSED);
            }
        }
    }
}

/// The text between `start` and `end` (inclusive, reading order) on `screen`.
/// Trailing blanks on each row are dropped and rows join with `\n`; the
/// right half of a wide character is skipped, not copied as a space.
pub fn text_in(screen: &vt100::Screen, start: (u16, u16), end: (u16, u16)) -> String {
    let (rows, cols) = screen.size();
    if rows == 0 || cols == 0 {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::new();
    for row in start.1..=end.1.min(rows - 1) {
        let from = if row == start.1 { start.0 } else { 0 };
        let to = if row == end.1 { end.0 } else { cols - 1 };
        let mut line = String::new();
        for col in from..=to.min(cols - 1) {
            match screen.cell(row, col) {
                Some(cell) if cell.is_wide_continuation() => {}
                Some(cell) if !cell.contents().is_empty() => line.push_str(cell.contents()),
                _ => line.push(' '),
            }
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n").trim_end_matches('\n').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(text: &str) -> vt100::Parser {
        let mut p = vt100::Parser::new(5, 20, 0);
        p.process(text.as_bytes());
        p
    }

    fn sel(a: (u16, u16), h: (u16, u16)) -> Selection {
        Selection {
            session: "s".into(),
            anchor: a,
            head: h,
        }
    }

    #[test]
    fn a_click_selects_nothing_and_a_backwards_drag_is_reordered() {
        assert!(sel((3, 1), (3, 1)).is_empty());
        assert!(!sel((3, 1), (3, 1)).contains(3, 1));
        let back = sel((6, 2), (1, 0));
        assert_eq!(back.ordered(), ((1, 0), (6, 2)));
        assert!(back.contains(10, 1), "middle rows are fully selected");
        assert!(back.contains(1, 0) && !back.contains(0, 0));
        assert!(back.contains(6, 2) && !back.contains(7, 2));
    }

    #[test]
    fn text_spans_rows_trims_trailing_blanks_and_honours_the_end_columns() {
        let p = screen("hello world\r\nsecond line   \r\nthird");
        let s = p.screen();
        assert_eq!(text_in(s, (6, 0), (5, 1)), "world\nsecond");
        assert_eq!(
            text_in(s, (0, 0), (19, 2)),
            "hello world\nsecond line\nthird"
        );
        assert_eq!(text_in(s, (2, 1), (2, 1)), "c");
    }

    #[test]
    fn a_wide_character_is_copied_once_not_followed_by_a_space() {
        let p = screen("a世b");
        assert_eq!(text_in(p.screen(), (0, 0), (4, 0)), "a世b");
    }

    #[test]
    fn local_cell_clamps_a_drag_that_leaves_the_tile() {
        let inner = Rect::new(10, 5, 20, 8);
        assert_eq!(local_cell(inner, 12, 6), (2, 1));
        assert_eq!(local_cell(inner, 2, 0), (0, 0));
        assert_eq!(local_cell(inner, 99, 99), (19, 7));
    }
}
