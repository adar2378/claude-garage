//! Tile PTY registry (port of `tui/lib/ui/tile_registry.dart`, grown around
//! [`TileClient`]): attach clients live here, keyed by session id, created
//! and dropped by [`TileRegistry::sync`]ing against the wall state's gridded
//! live set — never inside the draw. Dropping a client is detach-first
//! (TileClient::Drop), so reconciliation can never inject into a pane.
//!
//! Reattach loop (spec tui-wall "Tile PTY death recovery"): when an attach
//! PTY exits while the daemon still lists the session live, restart after
//! 500 ms, at most 4 attempts — then the tile shows a dead placeholder with
//! the reason. An attach that stayed up 5s counts as healthy and resets the
//! budget, so an inner session recreated hours later reattaches fresh.
//! Initial attaches are staggered 50 ms apart to keep the first frames
//! cheap.
//!
//! The per-tile frozen scrollback view (capture-pane snapshot, task 4.5)
//! also lives on the entry so it dies with the tile.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::pty::TileClient;
use crate::runtime::{AppEvent, EventSender};
use crate::ui::scroll::ScrollModel;

/// Pure retry policy for the reattach loop (port of the Dart
/// `ReattachPolicy`) — extracted so the backoff rules unit-test without
/// processes or timers.
pub struct ReattachPolicy {
    pub max_attempts: u32,
    pub retry_delay: Duration,
    /// An attach that survived this long counts as healthy: the next exit
    /// starts a fresh attempt budget instead of inheriting old failures.
    pub healthy_uptime: Duration,
    attempts: u32,
}

impl Default for ReattachPolicy {
    fn default() -> ReattachPolicy {
        ReattachPolicy {
            max_attempts: 4,
            retry_delay: Duration::from_millis(500),
            healthy_uptime: Duration::from_secs(5),
            attempts: 0,
        }
    }
}

impl ReattachPolicy {
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Record an exit after `uptime` of running. Returns the delay before
    /// the next restart, or `None` when the budget is exhausted (dead
    /// placeholder).
    pub fn on_exit(&mut self, uptime: Duration) -> Option<Duration> {
        if uptime >= self.healthy_uptime {
            self.attempts = 0;
        }
        if self.attempts >= self.max_attempts {
            return None;
        }
        self.attempts += 1;
        Some(self.retry_delay)
    }
}

/// Attach-stagger clock (pure): each new attach fires >= `stagger` after the
/// one before it, never in the past.
pub fn stagger_at(next_attach_at: &mut Instant, now: Instant, stagger: Duration) -> Instant {
    let at = (*next_attach_at).max(now);
    *next_attach_at = at + stagger;
    at
}

/// One tile's frozen scrollback view: the paging model plus the rendered
/// capture-pane snapshot and the `+N lines` counter.
pub struct FrozenTile {
    pub model: ScrollModel,
    pub parser: vt100::Parser,
    pub new_lines: i64,
    /// Throttle for the `+N lines` refresh (a tmux call).
    pub counted_at: Instant,
}

enum Slot {
    /// Scheduled attach (initial stagger or a retry delay).
    Pending { at: Instant },
    Attached(TileClient),
    /// Reattach budget exhausted — the placeholder names the reason.
    Dead(String),
}

struct Entry {
    slot: Slot,
    policy: ReattachPolicy,
    started_at: Instant,
    /// Desired inner size (cols, rows) from the last layout pass — applied
    /// on attach and on change (TIOCSWINSZ via TileClient::resize).
    size: (u16, u16),
    frozen: Option<FrozenTile>,
}

pub struct TileRegistry {
    entries: HashMap<String, Entry>,
    tx: EventSender,
    next_attach_at: Instant,
    stagger: Duration,
    scrollback: usize,
}

impl TileRegistry {
    pub fn new(tx: EventSender) -> TileRegistry {
        TileRegistry {
            entries: HashMap::new(),
            tx,
            next_attach_at: Instant::now(),
            stagger: Duration::from_millis(50),
            scrollback: 0,
        }
    }

    /// Reconcile against the ids that should have live PTYs (the gridded,
    /// live sessions): schedule missing attaches (staggered), drop clients
    /// whose id left the set (detach-first via Drop). Restorable sessions
    /// must not be passed in — they render placeholders, no PTY is ever
    /// spawned for them.
    pub fn sync(&mut self, live_gridded_ids: &[String]) {
        self.entries.retain(|id, _| live_gridded_ids.contains(id));
        let now = Instant::now();
        for id in live_gridded_ids {
            if self.entries.contains_key(id) {
                continue;
            }
            let at = stagger_at(&mut self.next_attach_at, now, self.stagger);
            self.entries.insert(
                id.clone(),
                Entry {
                    slot: Slot::Pending { at },
                    policy: ReattachPolicy::default(),
                    started_at: now,
                    size: (80, 24),
                    frozen: None,
                },
            );
        }
    }

    /// Push the tile's inner size from the latest layout pass. Applied to a
    /// running client immediately (TIOCSWINSZ, no-op when unchanged) and
    /// remembered for the next (re)attach.
    pub fn apply_size(&mut self, id: &str, cols: u16, rows: u16) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.size = (cols, rows);
            if let Slot::Attached(client) = &mut entry.slot {
                client.resize(cols, rows);
            }
        }
    }

    /// Drive pending attaches and exit detection. Returns true when any
    /// slot changed (the caller marks the frame dirty and re-checks the
    /// engaged tile).
    pub fn tick(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for (id, entry) in &mut self.entries {
            match &mut entry.slot {
                Slot::Pending { at } if now >= *at => {
                    entry.started_at = now;
                    let (cols, rows) = entry.size;
                    let tx = self.tx.clone();
                    match TileClient::new(id, cols, rows, self.scrollback, move || {
                        let _ = tx.send(AppEvent::TileOutput);
                    }) {
                        Ok(client) => entry.slot = Slot::Attached(client),
                        Err(e) => {
                            entry.slot = next_slot(
                                &mut entry.policy,
                                now,
                                Duration::ZERO,
                                format!("attach failed: {e}"),
                            );
                        }
                    }
                    changed = true;
                }
                Slot::Attached(client) => {
                    if let Some(code) = client.poll_exited() {
                        let uptime = now.saturating_duration_since(entry.started_at);
                        // Replace the slot first so the old client drops
                        // (detach-first; the client already exited, so the
                        // detach is a harmless no-op).
                        entry.slot = next_slot(
                            &mut entry.policy,
                            now,
                            uptime,
                            format!("attach exited (code {code})"),
                        );
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        changed
    }

    pub fn client_mut(&mut self, id: &str) -> Option<&mut TileClient> {
        match self.entries.get_mut(id).map(|e| &mut e.slot) {
            Some(Slot::Attached(client)) => Some(client),
            _ => None,
        }
    }

    pub fn client(&self, id: &str) -> Option<&TileClient> {
        match self.entries.get(id).map(|e| &e.slot) {
            Some(Slot::Attached(client)) => Some(client),
            _ => None,
        }
    }

    pub fn dead_reason(&self, id: &str) -> Option<&str> {
        match self.entries.get(id).map(|e| &e.slot) {
            Some(Slot::Dead(reason)) => Some(reason),
            _ => None,
        }
    }

    /// One rendered row of a tile's terminal as plain text, one char per
    /// cell (blank cells become spaces so byte-column == cell-column for
    /// the ASCII text URLs are made of). Reads the frozen snapshot when the
    /// tile is frozen, else the live screen. p14: feeds the click-on-URL
    /// router (spec tui-key-routing "Click opens links").
    pub fn row_text(&self, id: &str, row: u16) -> Option<String> {
        fn screen_row(screen: &vt100::Screen, row: u16) -> String {
            let (_, cols) = screen.size();
            let mut out = String::with_capacity(cols as usize);
            for col in 0..cols {
                match screen.cell(row, col) {
                    Some(cell) if !cell.contents().is_empty() => out.push_str(&cell.contents()),
                    _ => out.push(' '),
                }
            }
            out
        }
        let entry = self.entries.get(id)?;
        if let Some(frozen) = &entry.frozen {
            return Some(screen_row(frozen.parser.screen(), row));
        }
        match &entry.slot {
            Slot::Attached(client) => {
                let parser = client.parser().lock().unwrap();
                Some(screen_row(parser.screen(), row))
            }
            _ => None,
        }
    }

    pub fn frozen(&self, id: &str) -> Option<&FrozenTile> {
        self.entries.get(id).and_then(|e| e.frozen.as_ref())
    }

    pub fn frozen_mut(&mut self, id: &str) -> Option<&mut FrozenTile> {
        self.entries.get_mut(id).and_then(|e| e.frozen.as_mut())
    }

    pub fn set_frozen(&mut self, id: &str, frozen: Option<FrozenTile>) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.frozen = frozen;
        }
    }

    /// Refresh every frozen tile's `+N lines` counter from tmux, throttled
    /// to one query per `min_age` per tile. Returns true when any visible
    /// counter changed (the caller marks the frame dirty).
    pub fn refresh_frozen_counts(&mut self, min_age: Duration) -> bool {
        let now = Instant::now();
        let mut changed = false;
        for (id, entry) in &mut self.entries {
            let Some(frozen) = entry.frozen.as_mut() else {
                continue;
            };
            if now.saturating_duration_since(frozen.counted_at) < min_age {
                continue;
            }
            frozen.counted_at = now;
            if let Some(hist) = crate::ui::scroll::tmux_history_size(id) {
                let n = frozen.model.new_lines(hist);
                if n != frozen.new_lines {
                    frozen.new_lines = n;
                    changed = true;
                }
            }
        }
        changed
    }

    /// The tile's desired inner size from the last layout pass.
    pub fn size_of(&self, id: &str) -> Option<(u16, u16)> {
        self.entries.get(id).map(|e| e.size)
    }

    /// Engaged typing / wheel forwarding. Silently dropped while the tile
    /// has no running client (attaching or dead), same as the Dart writer.
    pub fn write(&mut self, id: &str, bytes: &[u8]) {
        if let Some(client) = self.client_mut(id) {
            let _ = client.write_bytes(bytes);
        }
    }
}

fn next_slot(policy: &mut ReattachPolicy, now: Instant, uptime: Duration, what: String) -> Slot {
    match policy.on_exit(uptime) {
        Some(delay) => Slot::Pending { at: now + delay },
        None => Slot::Dead(format!(
            "{what} — gave up after {} reattach attempts",
            policy.max_attempts
        )),
    }
}

#[cfg(test)]
mod tests {
    //! Port of the Dart `ReattachPolicy` tests plus the stagger clock.
    use super::*;

    #[test]
    fn budget_is_four_attempts_then_dead() {
        let mut p = ReattachPolicy::default();
        for i in 1..=4 {
            assert_eq!(
                p.on_exit(Duration::from_millis(100)),
                Some(Duration::from_millis(500)),
                "attempt {i}"
            );
        }
        assert_eq!(p.on_exit(Duration::from_millis(100)), None, "budget spent");
        assert_eq!(p.attempts(), 4);
    }

    #[test]
    fn a_healthy_uptime_resets_the_budget() {
        let mut p = ReattachPolicy::default();
        for _ in 0..4 {
            p.on_exit(Duration::from_millis(100));
        }
        // The attach that then survived 5s starts a fresh budget.
        assert_eq!(
            p.on_exit(Duration::from_secs(5)),
            Some(Duration::from_millis(500))
        );
        assert_eq!(p.attempts(), 1);
    }

    #[test]
    fn dead_reason_wording_matches_the_dart_placeholder() {
        let mut p = ReattachPolicy::default();
        for _ in 0..4 {
            p.on_exit(Duration::ZERO);
        }
        let now = Instant::now();
        match next_slot(&mut p, now, Duration::ZERO, "attach exited (code 1)".into()) {
            Slot::Dead(reason) => assert_eq!(
                reason,
                "attach exited (code 1) — gave up after 4 reattach attempts"
            ),
            _ => panic!("expected Dead"),
        }
    }

    #[test]
    fn stagger_spaces_attaches_and_never_schedules_in_the_past() {
        let now = Instant::now();
        let mut clock = now - Duration::from_secs(10); // long-idle registry
        let a = stagger_at(&mut clock, now, Duration::from_millis(50));
        let b = stagger_at(&mut clock, now, Duration::from_millis(50));
        let c = stagger_at(&mut clock, now, Duration::from_millis(50));
        assert_eq!(a, now, "first fires immediately, not in the past");
        assert_eq!(b, now + Duration::from_millis(50));
        assert_eq!(c, now + Duration::from_millis(100));
    }
}
