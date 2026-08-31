//! p8.4 kill-all removal confirm (port of
//! `tui/lib/state/workspace_remove.dart` — spec tui-key-routing "p8.4
//! kill-all removal confirm"): while the `X` remove arm is active, `K`
//! confirms the removal WITH session kill
//! (`DELETE /api/workspaces/<name>?sessions=kill`).
//!
//! The [`ArmedAction`] machine stays generic — `K` is a caller-level branch
//! on the armed state, expressed here as pure helpers so the wording and the
//! confirm decision unit-test without a live TUI.

use serde_json::Value;

use crate::state::armed_action::ArmedAction;

/// Caller-level `K` branch on the `X` remove arm: returns the armed
/// workspace name when the arm is live (consuming the arm — the caller then
/// fires the kill-remove), or `None` when `K` is an ordinary unbound key
/// (nothing armed, or the 3s window expired — an expired arm is disarmed
/// here so the stale target can't linger).
pub fn confirm_kill_target(armed_remove: &mut ArmedAction, now_ms: i64) -> Option<String> {
    let name = armed_remove.armed_id()?.to_owned();
    if armed_remove.press(&name, now_ms) {
        return Some(name); // confirmed + consumed
    }
    // press() re-armed an expired window — K must never (re-)arm, undo it.
    armed_remove.disarm();
    None
}

/// The `X` arm strip notice. With zero live sessions there is nothing for
/// `K` to kill, so the clause is omitted (p8.3's exact wording).
pub fn remove_arm_notice(name: &str, live_sessions: usize) -> String {
    let base = format!("press X again to remove {name} (sessions keep running)");
    if live_sessions == 0 {
        return base;
    }
    format!("{base} · K to also kill its {live_sessions} sessions")
}

/// Strip notice for a completed `K` confirm, from the daemon response body
/// (`{removed, killedSessions, failedSessions?}`). Failures are named so a
/// half-dead workspace is never reported as cleanly removed.
pub fn kill_remove_notice(name: &str, response: Option<&Value>) -> String {
    let killed = response
        .and_then(|r| r.get("killedSessions"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let failed: Vec<String> = response
        .and_then(|r| r.get("failedSessions"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|f| f.is_object())
                .map(|f| match f.get("id") {
                    Some(Value::String(id)) => id.clone(),
                    Some(other) => other.to_string(),
                    None => "null".to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    let base = format!("removed {name} · killed {killed} sessions");
    if failed.is_empty() {
        return base;
    }
    format!("{base} · failed to kill: {}", failed.join(", "))
}

#[cfg(test)]
mod tests {
    //! Port of `tui/test/workspace_remove_test.dart` — the caller-level `K`
    //! branch and the strip wording.
    use super::*;
    use crate::state::store::garage_command_for;
    use serde_json::json;
    use std::time::Duration;

    // ── confirmKillTarget (the K branch on the X arm) ───────────────────

    #[test]
    fn k_with_nothing_armed_confirms_nothing() {
        let mut armed = ArmedAction::default();
        assert_eq!(confirm_kill_target(&mut armed, 1000), None);
        assert_eq!(armed.armed_id(), None, "K must never arm");
    }

    #[test]
    fn k_confirms_the_armed_workspace_within_the_window_and_consumes_the_arm() {
        let mut armed = ArmedAction::default();
        assert!(!armed.press("proj", 1000)); // X arms
        assert_eq!(confirm_kill_target(&mut armed, 2500).as_deref(), Some("proj"));
        assert_eq!(armed.armed_id(), None, "confirm consumes the arm");
        assert_eq!(
            confirm_kill_target(&mut armed, 2600),
            None,
            "a second K after the confirm is unbound again"
        );
    }

    #[test]
    fn an_expired_arm_never_confirms_and_k_never_re_arms_it() {
        let mut armed = ArmedAction::new(Duration::from_secs(3));
        armed.press("proj", 1000);
        assert_eq!(confirm_kill_target(&mut armed, 4001), None, "window expired");
        assert_eq!(
            armed.armed_id(),
            None,
            "the stale arm is disarmed, not re-armed by K"
        );
    }

    #[test]
    fn after_an_x_x_confirm_the_machine_is_empty_so_a_trailing_k_is_unbound() {
        let mut armed = ArmedAction::default();
        armed.press("proj", 1000);
        assert!(armed.press("proj", 1500)); // X-X registry-only confirm
        assert_eq!(confirm_kill_target(&mut armed, 1600), None);
    }

    #[test]
    fn only_a_registered_workspace_can_ever_reach_a_k_confirm() {
        // The TUI's remove-press handler returns before press() for a group
        // with registered: false — modeled here as "no press happened".
        let mut armed = ArmedAction::default();
        assert_eq!(armed.armed_id(), None);
        assert_eq!(confirm_kill_target(&mut armed, 1000), None);
    }

    #[test]
    fn k_outside_an_arm_maps_to_no_garage_command_typing_hint_path() {
        assert_eq!(garage_command_for("K"), None);
    }

    // ── removeArmNotice wording ─────────────────────────────────────────

    #[test]
    fn live_sessions_append_the_k_clause_with_the_count() {
        assert_eq!(
            remove_arm_notice("proj", 2),
            "press X again to remove proj (sessions keep running) · K to also kill its 2 sessions"
        );
    }

    #[test]
    fn zero_live_sessions_omit_the_k_clause_p8_3_wording_unchanged() {
        assert_eq!(
            remove_arm_notice("proj", 0),
            "press X again to remove proj (sessions keep running)"
        );
    }

    // ── killRemoveNotice wording ────────────────────────────────────────

    #[test]
    fn clean_kill_reports_the_count() {
        let response = json!({
            "removed": "proj",
            "killedSessions": ["garage/proj/a", "garage/proj/b"],
        });
        assert_eq!(
            kill_remove_notice("proj", Some(&response)),
            "removed proj · killed 2 sessions"
        );
    }

    #[test]
    fn failures_are_named_never_silently_dropped() {
        let response = json!({
            "removed": "proj",
            "killedSessions": ["garage/proj/a"],
            "failedSessions": [{"id": "garage/proj/b", "error": "boom"}],
        });
        assert_eq!(
            kill_remove_notice("proj", Some(&response)),
            "removed proj · killed 1 sessions · failed to kill: garage/proj/b"
        );
    }

    #[test]
    fn a_missing_odd_body_still_yields_a_sane_notice() {
        assert_eq!(
            kill_remove_notice("proj", None),
            "removed proj · killed 0 sessions"
        );
    }
}
