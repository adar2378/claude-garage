//! Frozen scrollback over tmux history (spec tui-scrollback as amended by
//! the p9 delta; spike finding 1 is the reference implementation).
//!
//! tmux attach clients render on the alternate screen, so a tile's embedded
//! vt100 emulator never accumulates scrollback. The frozen view is therefore
//! seeded from tmux itself: one `capture-pane -e` snapshot anchored at an
//! ABSOLUTE history position (`#{history_size}`-based, immune to live growth
//! by construction), paged within. Returning to live resumes the live attach
//! view; tmux copy-mode is never entered on the underlying session.
//!
//! [`ScrollModel`] is the pure paging math (unit-tested); the tmux calls live
//! in [`tmux_history_size`] / [`capture_frozen`].

use std::process::Command;

/// Lines scrolled per mouse-wheel notch (the Dart `ScrollAnchor.wheelLines`).
pub const WHEEL_LINES: i64 = 3;

/// Lines scrolled per Shift+PageUp/PageDown press (one-line overlap).
pub fn page_lines(view_height: u16) -> i64 {
    i64::from(view_height.saturating_sub(1)).max(1)
}

/// Pure frozen-view paging state in ABSOLUTE tmux history coordinates.
///
/// `anchor` is `#{history_size}` at freeze time — the absolute index of the
/// visible top row the moment the view froze. `lines_up` is how far above
/// that anchor the view has scrolled (`0 < lines_up <= anchor`); the view's
/// top row sits at absolute index `anchor - lines_up`. Live output grows
/// history but can never move either number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollModel {
    anchor: i64,
    lines_up: i64,
}

impl ScrollModel {
    /// Freeze at `history_size` and scroll up by `lines`. Returns `None`
    /// when there is no history to scroll into (the tile stays live).
    pub fn freeze(history_size: i64, lines: i64) -> Option<ScrollModel> {
        let anchor = history_size.max(0);
        let lines_up = lines.clamp(0, anchor);
        (lines_up > 0).then_some(ScrollModel { anchor, lines_up })
    }

    /// Scroll further up, clamped at the very start of history.
    #[must_use]
    pub fn scroll_up(self, lines: i64) -> ScrollModel {
        ScrollModel {
            lines_up: (self.lines_up + lines).min(self.anchor),
            ..self
        }
    }

    /// Scroll toward live. Reaching (or passing) the anchor tail returns
    /// `None` — back to live-follow.
    #[must_use]
    pub fn scroll_down(self, lines: i64) -> Option<ScrollModel> {
        let lines_up = self.lines_up - lines;
        (lines_up > 0).then_some(ScrollModel { lines_up, ..self })
    }

    /// Absolute history index of the view's top row.
    pub fn top_abs(self) -> i64 {
        self.anchor - self.lines_up
    }

    /// Lines arrived since freezing — the `+N lines` affordance count.
    pub fn new_lines(self, history_size_now: i64) -> i64 {
        (history_size_now - self.anchor).max(0)
    }
}

fn tmux(args: &[&str]) -> Option<std::process::Output> {
    Command::new("tmux").args(args).env_remove("TMUX").output().ok()
}

/// Current tmux history size of a session (absolute line 0 = oldest retained
/// history line; the visible top row sits at index history_size).
pub fn tmux_history_size(session: &str) -> Option<i64> {
    let out = tmux(&[
        "display-message",
        "-p",
        "-t",
        &format!("={session}:"),
        "-F",
        "#{history_size}",
    ])?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Capture `rows` lines starting at absolute line `top_abs` from the
/// session, with escape sequences (`-e`), rendered into a fresh vt100 parser
/// sized to the tile. The `-S`/`-E` arguments are relative to the CURRENT
/// history size, recomputed per call — the absolute window stays fixed, so a
/// re-capture under live load renders identical content.
pub fn capture_frozen(session: &str, top_abs: i64, cols: u16, rows: u16) -> Option<vt100::Parser> {
    let hist = tmux_history_size(session)?;
    let s_rel = top_abs - hist; // <= 0 reaches into history
    let e_rel = s_rel + i64::from(rows) - 1;
    let out = tmux(&[
        "capture-pane",
        "-p",
        "-e",
        "-t",
        &format!("={session}:"),
        "-S",
        &s_rel.to_string(),
        "-E",
        &e_rel.to_string(),
    ])?;
    if !out.status.success() {
        return None;
    }
    let mut parser = vt100::Parser::new(rows, cols, 0);
    let mut first = true;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if !first {
            parser.process(b"\r\n");
        }
        first = false;
        parser.process(line.as_bytes());
        parser.process(b"\x1b[0m"); // don't bleed styles across lines
    }
    Some(parser)
}

#[cfg(test)]
mod tests {
    //! The paging model (port of the Dart `scroll_anchor_test` semantics,
    //! translated to the capture-pane absolute-anchor coordinates of the p9
    //! spec delta).
    use super::*;

    #[test]
    fn freeze_captures_the_anchor_and_scrolls_up() {
        let m = ScrollModel::freeze(500, 23).unwrap();
        assert_eq!(m.top_abs(), 477);
        assert_eq!(m.new_lines(500), 0);
    }

    #[test]
    fn freezing_with_no_history_stays_live() {
        assert_eq!(ScrollModel::freeze(0, 23), None);
        assert_eq!(ScrollModel::freeze(-5, 23), None);
        assert_eq!(ScrollModel::freeze(100, 0), None, "scroll by zero is a no-op");
    }

    #[test]
    fn scroll_up_clamps_at_the_start_of_history() {
        let m = ScrollModel::freeze(10, 23).unwrap();
        assert_eq!(m.top_abs(), 0, "clamped: only 10 lines of history exist");
        assert_eq!(m.scroll_up(100).top_abs(), 0);
    }

    #[test]
    fn paging_down_returns_to_live_at_the_anchor_tail() {
        let m = ScrollModel::freeze(500, 46).unwrap(); // two pages of 23
        let m = m.scroll_down(23).unwrap();
        assert_eq!(m.top_abs(), 477);
        assert_eq!(m.scroll_down(23), None, "reached the tail — live again");
    }

    #[test]
    fn overshooting_down_also_snaps_live() {
        let m = ScrollModel::freeze(500, 5).unwrap();
        assert_eq!(m.scroll_down(100), None);
    }

    #[test]
    fn live_growth_never_moves_the_view_only_the_counter() {
        let m = ScrollModel::freeze(500, 23).unwrap();
        let top_before = m.top_abs();
        // 300 more lines scroll off under load: absolute coordinates pin the
        // view; only the affordance count grows.
        assert_eq!(m.top_abs(), top_before);
        assert_eq!(m.new_lines(800), 300);
        assert_eq!(m.new_lines(400), 0, "clock-skewed shrink clamps to 0");
    }

    #[test]
    fn wheel_and_page_steps() {
        assert_eq!(WHEEL_LINES, 3);
        assert_eq!(page_lines(24), 23);
        assert_eq!(page_lines(1), 1);
        assert_eq!(page_lines(0), 1);
    }
}
