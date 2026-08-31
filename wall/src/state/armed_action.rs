//! Generic armed double-press state machine (port of
//! `tui/lib/state/armed_action.dart` — spec tui-key-routing "p8.1 session
//! lifecycle bindings" for the `x` close, "p8.3 workspace removal" for the
//! `X` remove): the first press on a key arms it, the second press on the
//! SAME key within the timeout confirms; any other keyboard key — or a press
//! aimed at a different target — disarms/re-arms without confirming. Pure
//! (time injected) so it unit-tests without timers; the caller owns the
//! strip notice and its TTL. Keys are opaque target identifiers (a session
//! id for `x`, a workspace name for `X`) — one machine instance per binding,
//! so arming one never confirms the other.

use std::time::Duration;

pub struct ArmedAction {
    pub timeout: Duration,
    armed_id: Option<String>,
    armed_at_ms: i64,
}

impl Default for ArmedAction {
    fn default() -> ArmedAction {
        ArmedAction::new(Duration::from_secs(3))
    }
}

impl ArmedAction {
    pub fn new(timeout: Duration) -> ArmedAction {
        ArmedAction {
            timeout,
            armed_id: None,
            armed_at_ms: 0,
        }
    }

    /// The target id currently armed, or `None`. (The arm may have expired —
    /// [`ArmedAction::press`] checks the clock, this getter is for rendering
    /// only.)
    pub fn armed_id(&self) -> Option<&str> {
        self.armed_id.as_deref()
    }

    /// A press aimed at `id` at `now_ms`. Returns true when this press
    /// CONFIRMS the action (same id, within the window) — the caller then
    /// performs the effect. False means the press (re-)armed: show the
    /// "press again" notice.
    pub fn press(&mut self, id: &str, now_ms: i64) -> bool {
        let confirmed = self.armed_id.as_deref() == Some(id)
            && now_ms - self.armed_at_ms <= self.timeout.as_millis() as i64;
        if confirmed {
            self.armed_id = None;
            return true;
        }
        self.armed_id = Some(id.to_owned());
        self.armed_at_ms = now_ms;
        false
    }

    /// Any other key disarms (spec: "any other key disarms").
    pub fn disarm(&mut self) {
        self.armed_id = None;
    }
}

/// The `x` close arming is one instance of the generic [`ArmedAction`]
/// machine — p8.3's `X` workspace removal reuses the same machine with
/// workspace-name keys. This alias keeps the original name (and its tests)
/// intact (port of `tui/lib/state/armed_close.dart`).
pub type ArmedClose = ArmedAction;

#[cfg(test)]
mod armed_action_tests {
    //! Port of `tui/test/armed_action_test.dart`.
    use super::*;

    #[test]
    fn workspace_name_keys_arm_and_confirm_like_session_ids() {
        let mut armed = ArmedAction::default();
        assert!(!armed.press("apexlabs", 1000));
        assert_eq!(armed.armed_id(), Some("apexlabs"));
        assert!(armed.press("apexlabs", 2500));
        assert_eq!(armed.armed_id(), None, "confirming consumes the arm");
    }

    #[test]
    fn a_press_aimed_at_a_different_workspace_re_arms_never_confirms() {
        let mut armed = ArmedAction::default();
        assert!(!armed.press("apexlabs", 1000));
        // Focus moved to another workspace — new target.
        assert!(!armed.press("garage", 1500));
        assert_eq!(armed.armed_id(), Some("garage"));
    }

    #[test]
    fn the_x_and_shift_x_machines_are_independent_instances() {
        let mut close = ArmedAction::default(); // `x`, keyed by session id
        let mut remove = ArmedAction::default(); // `X`, keyed by workspace name
        assert!(!close.press("garage/ws/a", 1000));
        // X after x must arm removal, not confirm anything.
        assert!(!remove.press("ws", 1100));
        // In the TUI the X keypress also disarms the close machine ("any
        // other key disarms") — after that, x must arm from scratch.
        close.disarm();
        assert!(!close.press("garage/ws/a", 1200));
        // The remove arm was untouched by the close machine's traffic.
        assert_eq!(remove.armed_id(), Some("ws"));
    }

    #[test]
    fn an_expired_arm_re_arms_instead_of_confirming() {
        let mut armed = ArmedAction::new(Duration::from_secs(3));
        armed.press("ws", 1000);
        assert!(!armed.press("ws", 4001), "window expired");
        assert!(armed.press("ws", 4500));
    }

    #[test]
    fn armed_close_is_an_alias_of_the_generic_machine() {
        // Compile-time fact in Rust; keep the runtime shape check anyway so
        // the alias can't silently become a distinct type.
        let mut armed: ArmedClose = ArmedAction::default();
        assert!(!armed.press("garage/ws/a", 1));
        assert_eq!(armed.armed_id(), Some("garage/ws/a"));
    }
}

#[cfg(test)]
mod armed_close_tests {
    //! Port of `tui/test/armed_close_test.dart` — the `x` double-press close
    //! instance (second `x` within 3s confirms; any other key disarms; a
    //! different session re-arms instead of confirming).
    use super::*;

    const A: &str = "garage/ws/a";
    const B: &str = "garage/ws/b";

    #[test]
    fn first_press_arms_second_press_within_the_window_confirms() {
        let mut armed = ArmedClose::default();
        assert!(!armed.press(A, 1000));
        assert_eq!(armed.armed_id(), Some(A));
        assert!(armed.press(A, 2000));
        assert_eq!(armed.armed_id(), None, "confirming consumes the arm");
    }

    #[test]
    fn a_press_after_the_3s_window_re_arms_instead_of_confirming() {
        let mut armed = ArmedClose::default();
        assert!(!armed.press(A, 1000));
        assert!(!armed.press(A, 4001), "window expired");
        assert!(armed.press(A, 4500), "the re-arm opened a new window");
    }

    #[test]
    fn exactly_at_the_window_edge_still_confirms() {
        let mut armed = ArmedClose::default();
        armed.press(A, 1000);
        assert!(armed.press(A, 4000));
    }

    #[test]
    fn an_x_aimed_at_a_different_session_re_arms_never_confirms() {
        let mut armed = ArmedClose::default();
        assert!(!armed.press(A, 1000));
        assert!(!armed.press(B, 1500), "focus moved — new target");
        assert_eq!(armed.armed_id(), Some(B));
        assert!(armed.press(B, 2000));
    }

    #[test]
    fn disarm_any_other_key_cancels_the_pending_close() {
        let mut armed = ArmedClose::default();
        armed.press(A, 1000);
        armed.disarm();
        assert_eq!(armed.armed_id(), None);
        assert!(!armed.press(A, 1100), "must arm from scratch");
    }

    #[test]
    fn confirm_never_fires_twice_without_a_fresh_arm() {
        let mut armed = ArmedClose::default();
        armed.press(A, 1000);
        assert!(armed.press(A, 1500));
        assert!(!armed.press(A, 1600), "the third x starts a new arming cycle");
    }
}
