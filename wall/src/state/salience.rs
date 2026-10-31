//! Salience ordering (port of `tui/lib/state/salience.dart` — spec
//! tui-triage: "Salience-first ordering"):
//!  - workspaces containing >=1 needs-input session sort before the rest,
//!    stable otherwise (registration/discovery order preserved within each
//!    bucket);
//!  - within a workspace, needs-input sessions sort before the rest, stable
//!    otherwise;
//!  - workspaces sharing a parent directory form a family: members stay
//!    contiguous, a family floats if any member is blocked, blocked members
//!    lead within it (spec tui-triage "Salience-first ordering"); a workspace
//!    whose own dir is another's parent is the family's head and leads it
//!    (after any blocked members), labelled by its own name;
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
    /// The family's head workspace basename (else the shared parent
    /// folder's), set on the first member of a family of >=2 workspaces; the
    /// rail draws it as a faint row above that member's header.
    pub family_label: Option<String>,
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

/// Family key: the group dir's parent, trailing slash stripped, lowercased
/// (macOS paths differ in case). `None` for a missing dir or one with no
/// parent, which makes the group its own singleton family.
fn family_key(dir: Option<&str>) -> Option<String> {
    let (parent, _) = dir?.trim_end_matches('/').rsplit_once('/')?;
    (!parent.is_empty()).then(|| parent.to_lowercase())
}

/// The dir itself, normalised like `family_key`: trailing slash stripped,
/// lowercased. `None` for a missing or empty dir.
fn own_key(dir: Option<&str>) -> Option<String> {
    let dir = dir?.trim_end_matches('/');
    (!dir.is_empty()).then(|| dir.to_lowercase())
}

/// Basename of the dir's parent, original case.
fn parent_basename(dir: Option<&str>) -> Option<String> {
    let (parent, _) = dir?.trim_end_matches('/').rsplit_once('/')?;
    let base = parent.rsplit('/').next()?;
    (!base.is_empty()).then(|| base.to_owned())
}

/// Basename of the dir itself, original case.
fn own_basename(dir: Option<&str>) -> Option<String> {
    let base = dir?.trim_end_matches('/').rsplit('/').next()?;
    (!base.is_empty()).then(|| base.to_owned())
}

/// A family under construction: its key, members flagged `is_head`.
type Family = (Option<String>, Vec<(bool, WorkspaceGroup)>);

/// Reorder `groups` (insertion order) so family members are contiguous:
/// families ordered by their first member, members in insertion order.
/// A group whose own dir is another group's parent is a head: it joins its
/// children's family instead of its parent's (no chaining: a head that is
/// also a child stays the head of its own family). Blocked families float,
/// blocked members lead within a family, then the head, and the first member
/// of each family of >=2 gets `family_label` (the head's name, else the
/// shared parent's).
fn arrange_families(groups: Vec<WorkspaceGroup>) -> Vec<WorkspaceGroup> {
    let child_keys: Vec<Option<String>> = groups
        .iter()
        .map(|g| family_key(g.dir.as_deref()))
        .collect();
    let mut families: Vec<Family> = Vec::new();
    for (i, group) in groups.into_iter().enumerate() {
        let own = own_key(group.dir.as_deref());
        let is_head = own.as_ref().is_some_and(|o| {
            child_keys
                .iter()
                .enumerate()
                .any(|(j, k)| j != i && k.as_ref() == Some(o))
        });
        let key = if is_head { own } else { child_keys[i].clone() };
        match key
            .as_ref()
            .and_then(|k| families.iter().position(|(fk, _)| fk.as_ref() == Some(k)))
        {
            Some(i) => families[i].1.push((is_head, group)),
            None => families.push((key, vec![(is_head, group)])),
        }
    }
    let families = stable_partition(families, |(_, members)| {
        members.iter().any(|(_, g)| g.has_needs_input())
    });
    let mut out = Vec::new();
    for (_, mut members) in families {
        // stable: ties keep insertion order
        members.sort_by_key(|(is_head, g)| (!g.has_needs_input(), !*is_head));
        if members.len() >= 2 {
            let label = match members.iter().find(|(h, _)| *h) {
                Some((_, head)) => own_basename(head.dir.as_deref()),
                None => parent_basename(members[0].1.dir.as_deref()),
            };
            members[0].1.family_label = label;
        }
        out.extend(members.into_iter().map(|(_, g)| g));
    }
    out
}

pub fn build_groups(workspaces: &[WorkspaceInfo], sessions: &[WallSession]) -> Vec<WorkspaceGroup> {
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
            family_label: None,
            name,
        })
        .collect();

    arrange_families(groups)
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
                if let Some(candidate) = s.since
                    && incumbent.since.is_none_or(|i| candidate < i)
                {
                    best = Some(s);
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
            dir: Some(format!("/repos/{name}/{name}")),
            branch: None,
        }
    }

    fn session(workspace: &str, label: &str) -> WallSession {
        session_with(workspace, label, "working", None)
    }

    fn session_with(workspace: &str, label: &str, status: &str, since: Option<i64>) -> WallSession {
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
        let labels: Vec<&str> = groups[0]
            .sessions
            .iter()
            .map(|s| s.label.as_str())
            .collect();
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
        let labels: Vec<&str> = groups[0]
            .sessions
            .iter()
            .map(|s| s.label.as_str())
            .collect();
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

    // ── families ────────────────────────────────────────────────────────

    fn wsd(name: &str, dir: Option<&str>) -> WorkspaceInfo {
        WorkspaceInfo {
            name: name.to_owned(),
            dir: dir.map(str::to_owned),
            branch: None,
        }
    }

    fn labels(groups: &[WorkspaceGroup]) -> Vec<Option<&str>> {
        groups.iter().map(|g| g.family_label.as_deref()).collect()
    }

    fn real_registry() -> Vec<WorkspaceInfo> {
        let b = "/Users/saifulislam";
        vec![
            wsd(
                "cto-playground",
                Some(&format!("{b}/development/devmonks/cto-playground")),
            ),
            wsd(
                "et-mobile-app",
                Some(&format!(
                    "{b}/development/devmonks/elite-traders/et-mobile-app"
                )),
            ),
            wsd(
                "mobile-apps",
                Some(&format!("{b}/development/personal/mobile-apps")),
            ),
            wsd(
                "et-backend",
                Some(&format!(
                    "{b}/development/devmonks/elite-traders/et-backend"
                )),
            ),
            wsd(
                "et-admin",
                Some(&format!("{b}/Development/devmonks/elite-traders/et-admin")),
            ),
            wsd(
                "diary-app",
                Some(&format!("{b}/Development/personal/diary-app")),
            ),
        ]
    }

    #[test]
    fn real_registry_groups_siblings_and_labels_first_members() {
        let groups = build_groups(&real_registry(), &[]);
        assert_eq!(
            names(&groups),
            [
                "cto-playground",
                "et-mobile-app",
                "et-backend",
                "et-admin",
                "mobile-apps",
                "diary-app"
            ]
        );
        assert_eq!(
            labels(&groups),
            [
                None,
                Some("elite-traders"),
                None,
                None,
                Some("personal"),
                None
            ]
        );
    }

    fn elite_registry() -> Vec<WorkspaceInfo> {
        let b = "/Users/saifulislam";
        vec![
            wsd(
                "cto-playground",
                Some(&format!("{b}/development/devmonks/cto-playground")),
            ),
            wsd(
                "icon-monks",
                Some(&format!("{b}/development/devmonks/icon-monks")),
            ),
            wsd(
                "et-backend",
                Some(&format!(
                    "{b}/Development/devmonks/elite-traders/et-backend"
                )),
            ),
            wsd(
                "kowoby-statistics",
                Some(&format!("{b}/development/devmonks/kowoby-statistics")),
            ),
            wsd(
                "et-mobile-app",
                Some(&format!(
                    "{b}/Development/devmonks/elite-traders/et-mobile-app"
                )),
            ),
            wsd(
                "elite-traders",
                Some(&format!("{b}/development/devmonks/elite-traders")),
            ),
            wsd(
                "et-admin",
                Some(&format!("{b}/Development/devmonks/elite-traders/et-admin")),
            ),
        ]
    }

    #[test]
    fn a_head_joins_its_children_family_and_leads_it() {
        let groups = build_groups(&elite_registry(), &[]);
        assert_eq!(
            names(&groups),
            [
                "cto-playground",
                "icon-monks",
                "kowoby-statistics",
                "elite-traders",
                "et-backend",
                "et-mobile-app",
                "et-admin",
            ]
        );
        assert_eq!(
            labels(&groups),
            [
                Some("devmonks"),
                None,
                None,
                Some("elite-traders"),
                None,
                None,
                None
            ]
        );
    }

    #[test]
    fn a_blocked_child_leads_an_unblocked_head_and_takes_the_label() {
        let groups = build_groups(
            &[
                wsd("elite-traders", Some("/d/elite-traders")),
                wsd("et-a", Some("/d/elite-traders/et-a")),
                wsd("et-b", Some("/d/elite-traders/et-b")),
            ],
            &[session_with("et-b", "s", "needs-input", None)],
        );
        assert_eq!(names(&groups), ["et-b", "elite-traders", "et-a"]);
        assert_eq!(labels(&groups), [Some("elite-traders"), None, None]);
    }

    #[test]
    fn a_blocked_head_leads_among_blocked_members() {
        let groups = build_groups(
            &[
                wsd("et-a", Some("/d/elite-traders/et-a")),
                wsd("elite-traders", Some("/d/elite-traders")),
                wsd("et-b", Some("/d/elite-traders/et-b")),
            ],
            &[
                session_with("et-a", "s", "needs-input", None),
                session_with("elite-traders", "s", "needs-input", None),
            ],
        );
        assert_eq!(names(&groups), ["elite-traders", "et-a", "et-b"]);
        assert_eq!(labels(&groups), [Some("elite-traders"), None, None]);
    }

    #[test]
    fn a_head_with_a_trailing_slash_still_merges() {
        let groups = build_groups(
            &[
                wsd("kid", Some("/d/Head/kid")),
                wsd("Head", Some("/D/Head/")),
            ],
            &[],
        );
        assert_eq!(names(&groups), ["Head", "kid"]);
        assert_eq!(labels(&groups), [Some("Head"), None]);
    }

    #[test]
    fn nesting_does_not_chain_a_middle_head_owns_its_family() {
        let groups = build_groups(
            &[
                wsd("a", Some("/a")),
                wsd("b", Some("/a/b")),
                wsd("c", Some("/a/b/c")),
            ],
            &[],
        );
        // b heads {b, c}; a is a head too, but its only child (b) left for
        // b's own family, so family "/a" holds a alone and gets no label.
        assert_eq!(names(&groups), ["a", "b", "c"]);
        assert_eq!(labels(&groups), [None, Some("b"), None]);
    }

    #[test]
    fn family_merge_is_case_insensitive_label_keeps_first_members_case() {
        let groups = build_groups(
            &[wsd("a", Some("/X/Dev/a")), wsd("b", Some("/x/dev/b/"))],
            &[],
        );
        assert_eq!(labels(&groups), [Some("Dev"), None]);
    }

    #[test]
    fn dir_none_is_its_own_singleton_family() {
        let groups = build_groups(
            &[wsd("a", None), wsd("b", None), wsd("c", Some("/r/c"))],
            &[],
        );
        assert_eq!(names(&groups), ["a", "b", "c"]);
        assert_eq!(labels(&groups), [None, None, None]);
    }

    #[test]
    fn a_family_floats_when_one_member_is_blocked() {
        let groups = build_groups(
            &[
                wsd("x", Some("/r/x")),
                wsd("f1", Some("/p/f/f1")),
                wsd("f2", Some("/p/f/f2")),
            ],
            &[session_with("f2", "s", "needs-input", None)],
        );
        // blocked member leads its family, the family leads the rail
        assert_eq!(names(&groups), ["f2", "f1", "x"]);
        assert_eq!(labels(&groups), [Some("f"), None, None]);
    }

    #[test]
    fn synthesized_groups_join_a_family_by_their_session_dir() {
        let mut stray = session("stray", "s");
        stray.dir = Some("/p/f/stray".to_owned());
        let groups = build_groups(
            &[wsd("f1", Some("/p/f/f1")), wsd("x", Some("/r/x"))],
            &[stray],
        );
        assert_eq!(names(&groups), ["f1", "stray", "x"]);
        assert_eq!(labels(&groups), [Some("f"), None, None]);
        assert!(!groups[1].registered);
    }

    // ── jumpTarget ──────────────────────────────────────────────────────

    #[test]
    fn picks_the_needs_input_session_with_the_oldest_since_across_workspaces() {
        let sessions = [
            session_with("a", "young", "needs-input", Some(3000)),
            session_with("b", "old", "needs-input", Some(1000)),
            session_with("a", "busy", "working", Some(500)),
        ];
        assert_eq!(
            jump_target(&sessions).map(|s| s.label.as_str()),
            Some("old")
        );
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
        assert_eq!(
            jump_target(&sessions).map(|s| s.label.as_str()),
            Some("known")
        );

        let sessions = [
            session_with("a", "first", "needs-input", None),
            session_with("a", "second", "needs-input", None),
        ];
        assert_eq!(
            jump_target(&sessions).map(|s| s.label.as_str()),
            Some("first")
        );
    }
}
