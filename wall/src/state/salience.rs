//! Salience ordering (port of `tui/lib/state/salience.dart` — spec
//! tui-triage: "Salience-first ordering"):
//!  - workspaces containing >=1 needs-input session sort before the rest,
//!    stable otherwise (registration/discovery order preserved within each
//!    bucket);
//!  - within a workspace, needs-input sessions sort before the rest, stable
//!    otherwise;
//!  - sessions whose workspace isn't registered still get a synthesized
//!    group (registered: false) so nothing is invisible.
//!
//! groups.js leans on JS Array#sort being stable; like the Dart port, the
//! two-bucket rank sort is implemented as a stable partition — identical
//! semantics for a boolean key.

use crate::api::models::WorkspaceInfo;
use crate::state::wall_state::WallSession;

/// One rail group: a workspace (registered or synthesized) with its sessions
/// in salience order.
#[derive(Clone, Debug)]
pub struct WorkspaceGroup {
    pub name: String,
    pub dir: Option<String>,
    pub branch: Option<String>,
    pub registered: bool,
    pub sessions: Vec<WallSession>,
}

impl WorkspaceGroup {
    pub fn has_needs_input(&self) -> bool {
        self.sessions.iter().any(WallSession::needs_input)
    }
}

/// Stable two-bucket sort: everything matching `first` in original order,
/// then the rest in original order.
fn stable_partition<T>(items: Vec<T>, first: impl Fn(&T) -> bool) -> Vec<T> {
    let (hits, rest): (Vec<T>, Vec<T>) = items.into_iter().partition(|item| first(item));
    let mut out = hits;
    out.extend(rest);
    out
}

pub fn build_groups(
    workspaces: &[WorkspaceInfo],
    sessions: &[WallSession],
) -> Vec<WorkspaceGroup> {
    // Insertion-ordered, like groups.js's Map: registered workspaces first in
    // registry order, synthesized groups after in discovery order.
    let mut order: Vec<String> = Vec::new();
    let mut buckets: Vec<(Option<&WorkspaceInfo>, Vec<WallSession>)> = Vec::new();

    for ws in workspaces {
        order.push(ws.name.clone());
        buckets.push((Some(ws), Vec::new()));
    }
    for s in sessions {
        match order.iter().position(|name| *name == s.workspace) {
            Some(i) => buckets[i].1.push(s.clone()),
            None => {
                order.push(s.workspace.clone());
                buckets.push((None, vec![s.clone()]));
            }
        }
    }

    let groups: Vec<WorkspaceGroup> = order
        .into_iter()
        .zip(buckets)
        .map(|(name, (ws, group_sessions))| WorkspaceGroup {
            dir: ws
                .and_then(|w| w.dir.clone())
                .or_else(|| group_sessions.first().and_then(|s| s.dir.clone())),
            branch: ws.and_then(|w| w.branch.clone()),
            registered: ws.is_some(),
            sessions: stable_partition(group_sessions, WallSession::needs_input),
            name,
        })
        .collect();

    stable_partition(groups, WorkspaceGroup::has_needs_input)
}

/// The `R` restore-all selection (spec tui-key-routing "p8.1 session
/// lifecycle bindings"): every restorable session in `workspace`, listing
/// order preserved. The caller issues one `POST /api/sessions/restore {id}`
/// per id in parallel, so one failure never blocks the rest.
pub fn restorable_session_ids(sessions: &[WallSession], workspace: &str) -> Vec<String> {
    sessions
        .iter()
        .filter(|s| s.workspace == workspace && !s.live())
        .map(|s| s.id.clone())
        .collect()
}

/// The `a`-jump target (spec tui-triage: "The `a` jump lands engaged"): the
/// longest-waiting needs-input session across all workspaces — oldest
/// `since`. `None` when nothing is blocked. A `None` `since` never beats a
/// known one; ties keep the earliest-listed session (stable).
pub fn jump_target(sessions: &[WallSession]) -> Option<&WallSession> {
    let mut best: Option<&WallSession> = None;
    for s in sessions {
        if !s.needs_input() {
            continue;
        }
        match best {
            None => best = Some(s),
            Some(incumbent) => {
                if let Some(candidate) = s.since {
                    if incumbent.since.is_none_or(|i| candidate < i) {
                        best = Some(s);
                    }
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    //! Port of `tui/test/salience_test.dart` — blocked-first at both levels,
    //! stable otherwise, synthesized groups for unregistered workspaces.
    use super::*;

    fn ws(name: &str) -> WorkspaceInfo {
        WorkspaceInfo {
            name: name.to_owned(),
            dir: Some(format!("/repos/{name}")),
            branch: None,
        }
    }

    fn session(workspace: &str, label: &str) -> WallSession {
        session_with(workspace, label, "working", None)
    }

    fn session_with(
        workspace: &str,
        label: &str,
        status: &str,
        since: Option<i64>,
    ) -> WallSession {
        WallSession {
            id: format!("garage/{workspace}/{label}"),
            workspace: workspace.to_owned(),
            label: label.to_owned(),
            dir: None,
            status: status.to_owned(),
            since,
            message: None,
            branch: None,
            worktree: false,
            title: None,
            context: None,
        }
    }

    fn names(groups: &[WorkspaceGroup]) -> Vec<&str> {
        groups.iter().map(|g| g.name.as_str()).collect()
    }

    // ── buildGroups ─────────────────────────────────────────────────────

    #[test]
    fn registry_order_is_preserved_when_nothing_is_blocked() {
        let groups = build_groups(
            &[ws("alpha"), ws("beta"), ws("gamma")],
            &[session("beta", "x"), session("alpha", "y")],
        );
        assert_eq!(names(&groups), ["alpha", "beta", "gamma"]);
    }

    #[test]
    fn a_blocked_workspace_bubbles_above_earlier_ones_others_unchanged() {
        let groups = build_groups(
            &[ws("a"), ws("b"), ws("c")],
            &[
                session("a", "s1"),
                session_with("b", "s2", "needs-input", None),
                session("c", "s3"),
            ],
        );
        assert_eq!(names(&groups), ["b", "a", "c"]);
    }

    #[test]
    fn two_blocked_workspaces_keep_their_relative_order_stable_within_the_blocked_bucket() {
        let groups = build_groups(
            &[ws("a"), ws("b"), ws("c"), ws("d")],
            &[
                session_with("b", "s", "needs-input", None),
                session_with("d", "s", "needs-input", None),
            ],
        );
        assert_eq!(names(&groups), ["b", "d", "a", "c"]);
    }

    #[test]
    fn within_a_workspace_needs_input_sessions_sort_first_stable_otherwise() {
        let groups = build_groups(
            &[ws("a")],
            &[
                session_with("a", "one", "working", None),
                session_with("a", "two", "needs-input", None),
                session_with("a", "three", "done", None),
                session_with("a", "four", "needs-input", None),
            ],
        );
        assert_eq!(groups.len(), 1);
        let labels: Vec<&str> = groups[0].sessions.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["two", "four", "one", "three"]);
    }

    #[test]
    fn unblocked_sessions_keep_listing_order_stable_within_the_unblocked_bucket() {
        let groups = build_groups(
            &[ws("a")],
            &[
                session_with("a", "one", "done", None),
                session_with("a", "two", "idle", None),
                session_with("a", "three", "working", None),
            ],
        );
        assert_eq!(groups.len(), 1);
        let labels: Vec<&str> = groups[0].sessions.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["one", "two", "three"]);
    }

    #[test]
    fn a_session_in_an_unregistered_workspace_gets_a_synthesized_group() {
        let stray = WallSession {
            id: "garage/stray/s2".to_owned(),
            workspace: "stray".to_owned(),
            label: "s2".to_owned(),
            dir: Some("/tmp/stray".to_owned()),
            status: "working".to_owned(),
            since: None,
            message: None,
            branch: None,
            worktree: false,
            title: None,
            context: None,
        };
        let groups = build_groups(&[ws("a")], &[session("a", "s1"), stray]);
        assert_eq!(names(&groups), ["a", "stray"]);
        let stray_group = groups.last().unwrap();
        assert!(!stray_group.registered);
        assert_eq!(stray_group.dir.as_deref(), Some("/tmp/stray"));
        assert!(groups.first().unwrap().registered);
    }

    #[test]
    fn a_registered_workspace_with_no_sessions_still_appears() {
        let groups = build_groups(&[ws("empty")], &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "empty");
        assert!(groups[0].sessions.is_empty());
        assert!(!groups[0].has_needs_input());
    }

    #[test]
    fn empty_inputs_produce_no_groups() {
        assert!(build_groups(&[], &[]).is_empty());
    }

    // ── jumpTarget ──────────────────────────────────────────────────────

    #[test]
    fn picks_the_needs_input_session_with_the_oldest_since_across_workspaces() {
        let sessions = [
            session_with("a", "young", "needs-input", Some(3000)),
            session_with("b", "old", "needs-input", Some(1000)),
            session_with("a", "busy", "working", Some(500)),
        ];
        assert_eq!(jump_target(&sessions).map(|s| s.label.as_str()), Some("old"));
    }

    #[test]
    fn ignores_non_blocked_sessions_entirely() {
        let sessions = [
            session_with("a", "w", "working", Some(1)),
            session_with("a", "d", "done", Some(2)),
            session_with("a", "r", "restorable", None),
        ];
        assert!(jump_target(&sessions).is_none());
    }

    #[test]
    fn a_null_since_never_beats_a_known_one_all_null_keeps_the_first_listed() {
        let sessions = [
            session_with("a", "unknown", "needs-input", None),
            session_with("a", "known", "needs-input", Some(99)),
        ];
        assert_eq!(jump_target(&sessions).map(|s| s.label.as_str()), Some("known"));

        let sessions = [
            session_with("a", "first", "needs-input", None),
            session_with("a", "second", "needs-input", None),
        ];
        assert_eq!(jump_target(&sessions).map(|s| s.label.as_str()), Some("first"));
    }
}
