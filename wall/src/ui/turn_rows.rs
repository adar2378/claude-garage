//! Where a turn ended, in scrollback rows (p25 turn-markers).
//!
//! A plugin tells the wall that a turn happened with a `tile.gutter` item
//! `{turn: <at>}`, where `<at>` is the `at` (epoch ms) of the daemon's
//! `session.turn` event. Only the wall knows the scrollback it shows, so the
//! wall turns that into an absolute row the first time it draws the item on a
//! live tile: the tile's history size plus the cursor row, i.e. the bottom of
//! what the user saw when the turn ended. After that the row is remembered, so
//! a marker stays on its line while output scrolls past it.
//!
//! A turn first seen long after it happened (the tile was not on screen then)
//! is not placed: guessing would put the marker on the wrong line, so it is
//! remembered as "unplaceable" and never drawn.

use std::collections::{BTreeMap, HashMap};

use crate::slots::SlotItem;

/// How late after its `at` a turn may still be placed at the bottom of the
/// live tile. The plugin pushes within a second; the rest is for a slow
/// redraw or a tile that had to attach first.
pub const MAX_AGE_MS: i64 = 10_000;
/// Remembered turns per session; the oldest are forgotten first.
const PER_SESSION: usize = 256;

#[derive(Default)]
pub struct TurnRows {
    /// session id, then turn: the row, or `None` for "too old to place".
    rows: HashMap<String, BTreeMap<u64, Option<u64>>>,
}

impl TurnRows {
    /// The row of `turn`, placing it now if it is new. `live_row` is the row
    /// the bottom of the live tile is on (`None` while the tile is scrolled
    /// back or its history size is not known yet: placing waits).
    pub fn row(
        &mut self,
        session: &str,
        turn: u64,
        now_ms: i64,
        live_row: Option<u64>,
    ) -> Option<u64> {
        let turns = self.rows.entry(session.to_owned()).or_default();
        if let Some(known) = turns.get(&turn) {
            return *known;
        }
        let age = now_ms.saturating_sub(i64::try_from(turn).unwrap_or(i64::MAX));
        let placed = if age > MAX_AGE_MS {
            None
        } else {
            // Young but not placeable yet: ask again next frame.
            let row = live_row?;
            Some(row)
        };
        turns.insert(turn, placed);
        while turns.len() > PER_SESSION {
            turns.pop_first();
        }
        placed
    }

    /// Forget sessions that no longer exist.
    pub fn retain_sessions(&mut self, live: impl Fn(&str) -> bool) {
        self.rows.retain(|id, _| live(id));
    }

    /// Give every turn-addressed item its row (items whose turn cannot be
    /// placed keep no row and so are not drawn).
    pub fn resolve(
        &mut self,
        session: &str,
        items: &mut [SlotItem],
        now_ms: i64,
        live_row: Option<u64>,
    ) {
        for item in items {
            if let (None, Some(turn)) = (item.gutter_row, item.gutter_turn) {
                item.gutter_row = self.row(session, turn, now_ms, live_row);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn marker(turn: u64) -> SlotItem {
        let mut it = SlotItem::new(vec![], 0);
        it.gutter_turn = Some(turn);
        it
    }

    #[test]
    fn a_fresh_turn_is_placed_at_the_live_bottom_once_and_then_stays_put() {
        let mut rows = TurnRows::default();
        let turn = NOW as u64 - 500;
        assert_eq!(rows.row("s", turn, NOW, Some(120)), Some(120));
        // Output has scrolled the bottom on; the marker stays on its line.
        assert_eq!(rows.row("s", turn, NOW + 3000, Some(150)), Some(120));
        // Even with the tile scrolled back (no live row), the known row stands.
        assert_eq!(rows.row("s", turn, NOW + 4000, None), Some(120));
    }

    #[test]
    fn a_young_turn_waits_while_the_tile_cannot_say_where_it_is() {
        let mut rows = TurnRows::default();
        let turn = NOW as u64 - 100;
        assert_eq!(rows.row("s", turn, NOW, None), None, "history unknown yet");
        assert_eq!(
            rows.row("s", turn, NOW + 200, Some(40)),
            Some(40),
            "placed once it is known"
        );
    }

    #[test]
    fn a_turn_seen_too_late_is_never_guessed_at() {
        let mut rows = TurnRows::default();
        let turn = NOW as u64 - MAX_AGE_MS as u64 - 1;
        assert_eq!(rows.row("s", turn, NOW, Some(99)), None);
        assert_eq!(
            rows.row("s", turn, NOW + 1, Some(99)),
            None,
            "and not later either"
        );
    }

    #[test]
    fn sessions_are_separate_and_forgotten_when_they_go() {
        let mut rows = TurnRows::default();
        let turn = NOW as u64;
        assert_eq!(rows.row("a", turn, NOW, Some(1)), Some(1));
        assert_eq!(rows.row("b", turn, NOW, Some(2)), Some(2));
        rows.retain_sessions(|id| id == "b");
        assert_eq!(
            rows.row("a", turn, NOW, Some(7)),
            Some(7),
            "a was forgotten, so it is placed afresh"
        );
        assert_eq!(rows.row("b", turn, NOW, Some(8)), Some(2));
    }

    #[test]
    fn only_turn_items_without_a_row_are_resolved() {
        let mut rows = TurnRows::default();
        let mut with_row = marker(1);
        with_row.gutter_row = Some(5);
        let mut items = vec![marker(NOW as u64), with_row, SlotItem::new(vec![], 0)];
        rows.resolve("s", &mut items, NOW, Some(30));
        assert_eq!(
            items.iter().map(|i| i.gutter_row).collect::<Vec<_>>(),
            [Some(30), Some(5), None]
        );
    }

    #[test]
    fn the_oldest_turns_are_forgotten_past_the_cap() {
        let mut rows = TurnRows::default();
        for i in 0..(PER_SESSION as u64 + 10) {
            rows.row("s", NOW as u64 + i, NOW + i as i64, Some(i));
        }
        assert_eq!(rows.rows["s"].len(), PER_SESSION);
        assert!(!rows.rows["s"].contains_key(&(NOW as u64)));
    }
}
