//! Which widget revisions the wall already holds, so an SSE `widgets` frame
//! costs a fetch only when that plugin's `rev` actually moved.

use std::collections::HashMap;

#[derive(Default, Debug)]
pub struct RevTracker {
    seen: HashMap<String, u64>,
}

impl RevTracker {
    /// True when `plugin` at `rev` is news (and remembers it).
    pub fn should_fetch(&mut self, plugin: &str, rev: u64) -> bool {
        if self.seen.get(plugin) == Some(&rev) {
            return false;
        }
        self.seen.insert(plugin.to_owned(), rev);
        true
    }

    /// Note what a full snapshot carried.
    pub fn record_all(&mut self, revs: impl IntoIterator<Item = (String, u64)>) {
        self.seen = revs.into_iter().collect();
    }

    /// Forget everything: the next frame for any plugin fetches.
    pub fn reset(&mut self) {
        self.seen.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_moved_rev_is_fetched_and_a_reset_forgets() {
        let mut t = RevTracker::default();
        assert!(t.should_fetch("a", 1));
        assert!(!t.should_fetch("a", 1), "same rev: nothing to fetch");
        assert!(t.should_fetch("a", 2));
        assert!(t.should_fetch("b", 2), "revs are per plugin");
        t.record_all([("a".to_owned(), 5)]);
        assert!(!t.should_fetch("a", 5), "a full snapshot counts");
        assert!(t.should_fetch("b", 2), "and replaces what was known");
        t.reset();
        assert!(t.should_fetch("a", 5));
    }
}
