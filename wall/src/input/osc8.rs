//! OSC 8 hyperlinks (`ESC ] 8 ; params ; URI ST` ... text ... `ESC ] 8 ; ; ST`).
//!
//! Claude Code renders markdown links `[text](url)` this way: the URL is
//! never on screen, only the link text is. `vt100` hands OSC 8 to
//! [`vt100::Callbacks::unhandled_osc`], so [`LinkTracker`] (the parser's
//! callbacks) remembers which cells each link covers, and the click path asks
//! [`LinkTracker::link_at`].
//!
//! Invalidation: spans are stored by screen position, and positions go stale
//! when the screen scrolls or text is overwritten. Rather than track every
//! write, each span also stores the text of its cells as recorded, and a
//! lookup re-reads those cells from the live screen: a mismatch means the
//! span is gone. Scrolling, clearing and overwriting all self-heal this way
//! (stale spans are skipped, and evicted by newer spans or the cap). Known
//! limit: if different content with identical text lands exactly on a stale
//! span (a link reading "here" scrolls up one row while another "here" takes
//! its old cells), the old URL still matches until tmux repaints that row,
//! which records a fresh span over it. A resize drops everything; an
//! alternate-screen switch hides spans recorded on the other screen.

use std::collections::VecDeque;

use crate::input::links::sanitize_link;

/// Spans kept per tile; the oldest are dropped first.
const MAX_SPANS: usize = 256;

#[derive(Debug)]
struct Span {
    url: String,
    alt_screen: bool,
    /// `(row, col_start, col_end_exclusive)` runs, in reading order.
    runs: Vec<(u16, u16, u16)>,
    /// The cells' text at record time, run after run.
    text: String,
}

#[derive(Debug, Default)]
pub struct LinkTracker {
    /// The open link and where its text started.
    open: Option<(String, (u16, u16))>,
    spans: VecDeque<Span>,
}

/// A tile's parser: `vt100` with link tracking.
pub type TileParser = vt100::Parser<LinkTracker>;

pub fn new_parser(rows: u16, cols: u16, scrollback: usize) -> TileParser {
    vt100::Parser::new_with_callbacks(rows, cols, scrollback, LinkTracker::default())
}

/// The cursor as a cell position; a pending wrap (cursor parked one past the
/// last column) reads as the start of the next row.
fn norm(screen: &vt100::Screen, pos: (u16, u16)) -> (u16, u16) {
    let (_, cols) = screen.size();
    if pos.1 >= cols {
        (pos.0.saturating_add(1), 0)
    } else {
        pos
    }
}

fn cell_text(screen: &vt100::Screen, row: u16, col: u16) -> &str {
    screen.cell(row, col).map_or("", |c| c.contents())
}

fn runs_text(screen: &vt100::Screen, runs: &[(u16, u16, u16)]) -> String {
    let mut text = String::new();
    for &(row, a, b) in runs {
        for col in a..b {
            text.push_str(cell_text(screen, row, col));
        }
    }
    text
}

impl LinkTracker {
    /// Forget everything (resize: every position is meaningless).
    pub fn clear(&mut self) {
        self.open = None;
        self.spans.clear();
    }

    fn record(&mut self, screen: &vt100::Screen, url: String, from: (u16, u16), to: (u16, u16)) {
        let (rows, cols) = screen.size();
        let (from, to) = (norm(screen, from), norm(screen, to));
        if to <= from || from.0 >= rows {
            return;
        }
        let mut runs = Vec::new();
        for row in from.0..=to.0.min(rows - 1) {
            let a = if row == from.0 { from.1 } else { 0 };
            let b = if row == to.0 { to.1 } else { cols };
            if b > a {
                runs.push((row, a, b));
            }
        }
        let text = runs_text(screen, &runs);
        // A link over nothing visible is not clickable.
        if text.trim().is_empty() {
            return;
        }
        // New text supersedes whatever it was painted over (tmux repaints
        // re-emit the same link; this keeps the list from filling with
        // duplicates).
        self.spans.retain(|s| {
            !s.runs
                .iter()
                .any(|&(r, c, d)| runs.iter().any(|&(r2, a, b)| r == r2 && a < d && c < b))
        });
        if self.spans.len() >= MAX_SPANS {
            self.spans.pop_front();
        }
        self.spans.push_back(Span {
            url,
            alt_screen: screen.alternate_screen(),
            runs,
            text,
        });
    }

    /// The URL of the link under cell `(row, col)`, if the screen still shows
    /// the text it was recorded with.
    pub fn link_at(&self, screen: &vt100::Screen, row: u16, col: u16) -> Option<String> {
        let alt = screen.alternate_screen();
        self.spans
            .iter()
            .rev()
            .filter(|s| {
                s.runs
                    .iter()
                    .any(|&(r, a, b)| r == row && (a..b).contains(&col))
            })
            .find(|s| s.alt_screen == alt && runs_text(screen, &s.runs) == s.text)
            .map(|s| s.url.clone())
    }

    #[cfg(test)]
    fn span_count(&self) -> usize {
        self.spans.len()
    }
}

impl vt100::Callbacks for LinkTracker {
    fn unhandled_osc(&mut self, screen: &mut vt100::Screen, params: &[&[u8]]) {
        // `8 ; params ; URI` — the URI may itself hold `;`, which the
        // parser split on.
        let [b"8", _params, uri @ ..] = params else {
            return;
        };
        let uri = uri.join(&b';');
        let here = screen.cursor_position();
        // Any OSC 8 ends the link in progress.
        if let Some((url, from)) = self.open.take() {
            self.record(screen, url, from, here);
        }
        if !uri.is_empty()
            && let Some(url) = sanitize_link(&String::from_utf8_lossy(&uri))
        {
            self.open = Some((url, here));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(rows: u16, cols: u16, bytes: &str) -> TileParser {
        let mut p = new_parser(rows, cols, 0);
        p.process(bytes.as_bytes());
        p
    }

    fn at(p: &TileParser, row: u16, col: u16) -> Option<String> {
        p.callbacks().link_at(p.screen(), row, col)
    }

    #[test]
    fn link_text_resolves_on_every_cell_for_both_terminators_and_ids() {
        for (open, close) in [
            ("\x1b]8;;https://example.com/a\x07", "\x1b]8;;\x07"),
            ("\x1b]8;;https://example.com/a\x1b\\", "\x1b]8;;\x1b\\"),
            ("\x1b]8;id=7;https://example.com/a\x07", "\x1b]8;;\x07"),
            ("\x1b]8;id=7;https://example.com/a\x1b\\", "\x1b]8;;\x1b\\"),
        ] {
            let p = feed(3, 30, &format!("go {open}the docs{close} now"));
            for col in 3..11 {
                assert_eq!(
                    at(&p, 0, col).as_deref(),
                    Some("https://example.com/a"),
                    "{open:?} col {col}"
                );
            }
            assert_eq!(at(&p, 0, 2), None, "space before");
            assert_eq!(at(&p, 0, 11), None, "space after");
            assert_eq!(at(&p, 1, 4), None, "other row");
        }
    }

    #[test]
    fn a_link_that_wraps_covers_both_rows() {
        // 10 columns: the link text starts at column 8 and crosses the edge.
        let p = feed(
            3,
            10,
            "12345678\x1b]8;;https://w.dev/x\x07abcdefghij\x1b]8;;\x07",
        );
        for (row, col) in [(0, 8), (0, 9), (1, 0), (1, 7)] {
            assert_eq!(at(&p, row, col).as_deref(), Some("https://w.dev/x"));
        }
        assert_eq!(at(&p, 0, 7), None);
        assert_eq!(at(&p, 1, 8), None);
    }

    #[test]
    fn a_link_opened_with_the_cursor_parked_at_the_edge_starts_on_the_next_row() {
        let p = feed(3, 5, "abcde\x1b]8;;https://w.dev\x07xy\x1b]8;;\x07");
        assert_eq!(at(&p, 0, 4), None);
        assert_eq!(at(&p, 1, 0).as_deref(), Some("https://w.dev"));
        assert_eq!(at(&p, 1, 1).as_deref(), Some("https://w.dev"));
    }

    #[test]
    fn overwritten_or_scrolled_text_drops_the_link() {
        let link = "\x1b]8;;https://a.dev\x07link\x1b]8;;\x07";
        let mut p = feed(3, 20, link);
        assert_eq!(at(&p, 0, 1).as_deref(), Some("https://a.dev"));
        // Overwrite in place.
        p.process(b"\x1b[1;1Hplain");
        assert_eq!(at(&p, 0, 1), None);

        // Scroll the link off the top.
        let mut p = feed(3, 20, link);
        p.process(b"\r\n\r\n\r\n\r\nmore");
        assert_eq!(at(&p, 0, 1), None);

        // Cleared screen.
        let mut p = feed(3, 20, link);
        p.process(b"\x1b[2J");
        assert_eq!(at(&p, 0, 1), None);
    }

    #[test]
    fn a_repainted_link_replaces_the_old_span() {
        let link = "\x1b[1;1H\x1b]8;;https://a.dev\x07link\x1b]8;;\x07";
        let mut p = feed(3, 20, link);
        p.process(link.as_bytes());
        p.process(link.as_bytes());
        assert_eq!(p.callbacks().span_count(), 1);
        assert_eq!(at(&p, 0, 2).as_deref(), Some("https://a.dev"));
    }

    #[test]
    fn a_new_open_closes_the_previous_link() {
        let p = feed(
            2,
            30,
            "\x1b]8;;https://a.dev\x07one\x1b]8;;https://b.dev\x07two\x1b]8;;\x07",
        );
        assert_eq!(at(&p, 0, 1).as_deref(), Some("https://a.dev"));
        assert_eq!(at(&p, 0, 4).as_deref(), Some("https://b.dev"));
    }

    #[test]
    fn urls_with_semicolons_survive() {
        let p = feed(2, 30, "\x1b]8;;https://a.dev/p;q=1\x07hi\x1b]8;;\x07");
        assert_eq!(at(&p, 0, 0).as_deref(), Some("https://a.dev/p;q=1"));
    }

    #[test]
    fn only_web_links_are_kept() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ssh://host",
            "https://",
        ] {
            let p = feed(2, 30, &format!("\x1b]8;;{url}\x07hi\x1b]8;;\x07"));
            assert_eq!(at(&p, 0, 0), None, "{url}");
        }
    }

    #[test]
    fn spans_are_bounded_and_drop_the_oldest() {
        let mut p = new_parser(20, 20, 0);
        for i in 0..(MAX_SPANS + 40) {
            let (row, col) = (1 + i / 20, 1 + i % 20);
            p.process(
                format!("\x1b[{row};{col}H\x1b]8;;https://a.dev/{i}\x07x\x1b]8;;\x07").as_bytes(),
            );
        }
        assert_eq!(p.callbacks().span_count(), MAX_SPANS);
        assert_eq!(at(&p, 0, 0), None, "the oldest was dropped");
        let last = MAX_SPANS + 39;
        assert_eq!(
            at(&p, (last / 20) as u16, (last % 20) as u16),
            Some(format!("https://a.dev/{last}"))
        );
    }
}
