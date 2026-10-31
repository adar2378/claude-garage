//! Port of `daemon/src/status.js`: the in-memory status store and its transition rules.
//!
//! Four states: needs-input | working | done | idle. `set` is the single
//! write path. There is no auto-decay: `done` and `needs-input` hold until a
//! later `set` changes them or `drop_session` removes the entry. `since` is
//! the epoch ms of the last real state change. Only needs-input carries a
//! message; a messageless same-state write keeps a hook-set message, and any
//! transition away from needs-input clears it.
//!
//! Real transitions are published on the `EventBus` (replaces `statusEvents`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use garage_protocol::SessionStatus;

use crate::bus::{DaemonEvent, EventBus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub state: SessionStatus,
    /// Epoch ms the current state began; `None` for a never-signaled id.
    pub since: Option<i64>,
    pub message: Option<String>,
}

#[derive(Clone)]
pub struct StatusStore {
    map: Arc<Mutex<HashMap<String, StatusEntry>>>,
    bus: EventBus,
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl StatusStore {
    pub fn new(bus: EventBus) -> Self {
        Self { map: Arc::default(), bus }
    }

    /// Unknown/never-signaled sessions read as idle.
    pub fn get(&self, id: &str) -> SessionStatus {
        self.map.lock().unwrap().get(id).map(|e| e.state).unwrap_or(SessionStatus::Idle)
    }

    /// True only when the store holds a real entry (unlike `get`, which reads
    /// idle for an unobserved id).
    pub fn has(&self, id: &str) -> bool {
        self.map.lock().unwrap().contains_key(id)
    }

    pub fn entry(&self, id: &str) -> StatusEntry {
        self.map
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or(StatusEntry { state: SessionStatus::Idle, since: None, message: None })
    }

    pub fn all(&self) -> HashMap<String, StatusEntry> {
        self.map.lock().unwrap().clone()
    }

    /// `message: None` is JS `undefined` (not given).
    pub fn set(&self, id: &str, state: SessionStatus, message: Option<&str>) {
        self.set_at(id, state, message, now_ms());
    }

    pub fn set_at(&self, id: &str, state: SessionStatus, message: Option<&str>, now: i64) {
        let event = {
            let mut map = self.map.lock().unwrap();
            let existing = map.get(id);
            let prev = existing.map(|e| e.state).unwrap_or(SessionStatus::Idle);
            let changed = prev != state;
            let since = if changed { now } else { existing.and_then(|e| e.since).unwrap_or(now) };
            let next_message = if state != SessionStatus::NeedsInput {
                None
            } else if let Some(m) = message {
                Some(m.to_owned())
            } else if prev == SessionStatus::NeedsInput {
                existing.and_then(|e| e.message.clone())
            } else {
                None
            };
            map.insert(id.to_owned(), StatusEntry { state, since: Some(since), message: next_message });
            changed.then(|| DaemonEvent::StatusTransition { id: id.to_owned(), from: prev, to: state, since: Some(since) })
        };
        // Publish outside the lock.
        if let Some(e) = event {
            self.bus.publish(e);
        }
    }

    /// A session disappeared from tmux; a future reuse of the id starts clean.
    pub fn drop_session(&self, id: &str) {
        self.map.lock().unwrap().remove(id);
    }

    /// Port of `applyAgentStatus` (poller.js): map `claude agents` status
    /// (`busy`/`waiting`/anything else) and never let the poller's coarse
    /// idle stomp a hook-sourced needs-input or done.
    pub fn apply_agent_status(&self, id: &str, agent_status: &str) {
        let mapped = match agent_status {
            "busy" => SessionStatus::Working,
            "waiting" => SessionStatus::NeedsInput,
            _ => SessionStatus::Idle,
        };
        let current = self.get(id);
        if mapped == SessionStatus::Idle && matches!(current, SessionStatus::NeedsInput | SessionStatus::Done) {
            return;
        }
        self.set(id, mapped, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SessionStatus::*;

    const ID: &str = "garage/test/one";
    const MSG: &str = "Claude needs your permission to use Bash";

    fn store() -> StatusStore {
        StatusStore::new(EventBus::default())
    }

    #[test]
    fn unknown_sessions_read_as_idle() {
        assert_eq!(store().get("garage/never/heard-of"), Idle);
    }

    #[test]
    fn set_stores_and_get_reads() {
        let s = store();
        s.set(ID, Working, None);
        assert_eq!(s.get(ID), Working);
    }

    #[test]
    fn transition_events_fire_once_per_actual_change() {
        let s = store();
        let mut rx = s.bus.subscribe();
        s.set(ID, Working, None);
        s.set(ID, Working, None);
        s.set(ID, NeedsInput, None);
        let mut seen = vec![];
        while let Ok(DaemonEvent::StatusTransition { from, to, .. }) = rx.try_recv() {
            seen.push((from, to));
        }
        assert_eq!(seen, [(Idle, Working), (Working, NeedsInput)]);
    }

    #[test]
    fn done_and_needs_input_hold_with_no_auto_decay() {
        let s = store();
        s.set_at(ID, Done, None, 0);
        assert_eq!(s.get(ID), Done); // no timer exists; nothing can decay it
        s.set_at(ID, NeedsInput, None, 1);
        assert_eq!(s.get(ID), NeedsInput);
    }

    #[test]
    fn done_only_clears_via_an_explicit_transition() {
        let s = store();
        s.set(ID, Done, None);
        s.set(ID, Working, None);
        assert_eq!(s.get(ID), Working);
    }

    #[test]
    fn drop_session_forgets_state() {
        let s = store();
        s.set(ID, Done, None);
        s.drop_session(ID);
        assert_eq!(s.get(ID), Idle);
        assert!(!s.has(ID));
    }

    #[test]
    fn entry_defaults_to_idle_null_for_unknown_ids() {
        assert_eq!(
            store().entry("garage/never/heard-of"),
            StatusEntry { state: Idle, since: None, message: None }
        );
    }

    #[test]
    fn hook_set_message_rides_needs_input() {
        let s = store();
        s.set(ID, NeedsInput, Some(MSG));
        assert_eq!(s.entry(ID).message.as_deref(), Some(MSG));
    }

    #[test]
    fn message_clears_on_transition_away_and_never_resurrects() {
        let s = store();
        s.set(ID, NeedsInput, Some(MSG));
        s.set(ID, Working, None);
        assert_eq!(s.entry(ID).message, None);
        s.set(ID, NeedsInput, None);
        assert_eq!(s.entry(ID).message, None);
    }

    #[test]
    fn messageless_needs_input_transition_leaves_message_unset() {
        let s = store();
        s.set(ID, NeedsInput, None);
        assert_eq!(s.get(ID), NeedsInput);
        assert_eq!(s.entry(ID).message, None);
    }

    #[test]
    fn messageless_same_state_write_keeps_hook_message() {
        let s = store();
        s.set(ID, NeedsInput, Some(MSG));
        s.set(ID, NeedsInput, None);
        assert_eq!(s.entry(ID).message.as_deref(), Some(MSG));
    }

    #[test]
    fn message_never_sticks_to_non_blocked_states() {
        let s = store();
        s.set(ID, Done, Some("should be dropped"));
        assert_eq!(s.entry(ID).message, None);
        s.set(ID, Working, Some("should also be dropped"));
        assert_eq!(s.entry(ID).message, None);
    }

    #[test]
    fn since_moves_on_real_transition_and_is_kept_on_repeat() {
        let s = store();
        s.set_at(ID, Working, None, 1000);
        let first = s.entry(ID).since;
        assert_eq!(first, Some(1000));
        s.set_at(ID, Working, None, 6000);
        assert_eq!(s.entry(ID).since, first);
        s.set_at(ID, Done, None, 11000);
        assert_eq!(s.entry(ID).since, Some(11000));
        assert_eq!(s.entry(ID).state, Done);
    }

    #[test]
    fn transition_event_carries_the_new_since() {
        let s = store();
        let mut rx = s.bus.subscribe();
        s.set_at(ID, Working, None, 42);
        assert_eq!(
            rx.try_recv().unwrap(),
            DaemonEvent::StatusTransition { id: ID.into(), from: Idle, to: Working, since: Some(42) }
        );
    }

    #[test]
    fn apply_agent_status_maps_and_never_downgrades_blocked_or_done() {
        let s = store();
        s.apply_agent_status(ID, "busy");
        assert_eq!(s.get(ID), Working);
        s.apply_agent_status(ID, "waiting");
        assert_eq!(s.get(ID), NeedsInput);
        s.apply_agent_status(ID, "idle");
        assert_eq!(s.get(ID), NeedsInput);
        s.set(ID, Done, None);
        s.apply_agent_status(ID, "idle");
        assert_eq!(s.get(ID), Done);
        s.apply_agent_status(ID, "busy");
        assert_eq!(s.get(ID), Working);
        s.apply_agent_status(ID, "idle");
        assert_eq!(s.get(ID), Idle);
        s.apply_agent_status(ID, "something-else");
        assert_eq!(s.get(ID), Idle);
    }
}
