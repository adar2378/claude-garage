//! Off-screen escalation (spec tui-triage "Off-screen escalation"; port of
//! `tui/lib/ui/escalation.dart`):
//!  - terminal bell (BEL, SSH-safe) once per session ENTERING needs-input,
//!    coalesced — a batch of simultaneous transitions rings once;
//!  - OSC 0 terminal title carrying the blocked count — `(N) garage` when
//!    N > 0, plain `garage` at zero — re-emitted on every count change (the
//!    very first update always claims the title).
//!
//! Pure policy: [`EscalationPolicy::update`] returns the escape string to
//! write (or `None`), so it unit-tests without a terminal. The runtime
//! writes the returned bytes straight to the terminal between frames — the
//! single state-owning loop both applies events and draws, so the sequences
//! can never interleave a frame paint.
//!
//! The visibility heartbeat (`POST /api/ui/visibility` every 30s, `false`
//! once on quit) lives in the runtime — it is IO through the API client, not
//! terminal policy.

use std::collections::HashSet;

use crate::state::wall_state::WallSession;

pub const BEL: &str = "\x07";

/// Interval for the `visible: true` heartbeat (same as the Dart TUI).
pub const HEARTBEAT_INTERVAL_MS: u64 = 30_000;

#[derive(Default)]
pub struct EscalationPolicy {
    blocked: HashSet<String>,
    title_count: Option<usize>,
}

/// Title text for a blocked count.
pub fn title_for(blocked: usize) -> String {
    if blocked > 0 {
        format!("({blocked}) garage")
    } else {
        "garage".to_owned()
    }
}

/// OSC 0 (icon + window title), BEL-terminated — the most widely supported
/// terminator, SSH-safe.
pub fn osc_title(title: &str) -> String {
    format!("\x1b]0;{title}{BEL}")
}

impl EscalationPolicy {
    /// Feed the current session snapshot. Returns at most one write per
    /// call: a single BEL when at least one session newly entered
    /// needs-input, plus the title sequence whenever the blocked count
    /// changed. A session already blocked in the previous snapshot never
    /// re-rings; one that left and re-entered does.
    pub fn update(&mut self, sessions: &[WallSession]) -> Option<String> {
        let now: HashSet<String> = sessions
            .iter()
            .filter(|s| s.needs_input())
            .map(|s| s.id.clone())
            .collect();
        let mut out = String::new();
        if now.difference(&self.blocked).next().is_some() {
            out.push_str(BEL);
        }
        if self.title_count != Some(now.len()) {
            out.push_str(&osc_title(&title_for(now.len())));
            self.title_count = Some(now.len());
        }
        self.blocked = now;
        (!out.is_empty()).then_some(out)
    }
}

#[cfg(test)]
mod tests {
    //! Port of `tui/test/escalation_test.dart` — coalesced bell, title on
    //! every count change, first-update title claim.
    use super::*;

    fn session(id: &str, status: &str) -> WallSession {
        WallSession {
            id: id.to_owned(),
            workspace: "ws".to_owned(),
            label: id.to_owned(),
            dir: None,
            status: status.to_owned(),
            since: None,
            message: None,
            branch: None,
            worktree: false,
            title: None,
        }
    }

    #[test]
    fn title_wording() {
        assert_eq!(title_for(0), "garage");
        assert_eq!(title_for(3), "(3) garage");
        assert_eq!(osc_title("garage"), "\x1b]0;garage\x07");
    }

    #[test]
    fn first_update_claims_the_title_even_with_nothing_blocked() {
        let mut p = EscalationPolicy::default();
        assert_eq!(
            p.update(&[session("a", "working")]),
            Some(osc_title("garage"))
        );
        assert_eq!(p.update(&[session("a", "working")]), None, "no re-emit");
    }

    #[test]
    fn a_new_blocked_session_rings_once_with_the_title() {
        let mut p = EscalationPolicy::default();
        p.update(&[]);
        assert_eq!(
            p.update(&[session("a", "needs-input")]),
            Some(format!("{BEL}{}", osc_title("(1) garage")))
        );
        // Still blocked: no re-ring, no title change.
        assert_eq!(p.update(&[session("a", "needs-input")]), None);
    }

    #[test]
    fn simultaneous_transitions_coalesce_to_one_bel() {
        let mut p = EscalationPolicy::default();
        p.update(&[]);
        let out = p
            .update(&[session("a", "needs-input"), session("b", "needs-input")])
            .unwrap();
        assert_eq!(out.matches(BEL).count(), 2, "one ring + the title's terminator");
        assert!(out.starts_with(BEL));
        assert!(out.ends_with(&osc_title("(2) garage")));
    }

    #[test]
    fn unblocking_updates_the_title_without_a_bell() {
        let mut p = EscalationPolicy::default();
        p.update(&[session("a", "needs-input")]);
        assert_eq!(
            p.update(&[session("a", "working")]),
            Some(osc_title("garage")),
            "count change re-emits the title, no leading BEL"
        );
    }

    #[test]
    fn leaving_and_re_entering_rings_again() {
        let mut p = EscalationPolicy::default();
        p.update(&[session("a", "needs-input")]);
        p.update(&[session("a", "working")]);
        let out = p.update(&[session("a", "needs-input")]).unwrap();
        assert!(out.starts_with(BEL));
    }

    #[test]
    fn a_swap_at_the_same_count_rings_without_a_title_change() {
        let mut p = EscalationPolicy::default();
        p.update(&[session("a", "needs-input")]);
        assert_eq!(
            p.update(&[session("b", "needs-input")]),
            Some(BEL.to_owned()),
            "b newly blocked rings; count unchanged keeps the title"
        );
    }
}
