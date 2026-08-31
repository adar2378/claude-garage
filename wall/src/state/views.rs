//! Named views within a workspace (spec tui-views; semantic port of
//! `ui/src/lib/views.js`'s `computeViews`/`viewOf`/`deriveViewName`).
//!
//! **Model** (design.md "State model"): every session implicitly belongs to
//! [`DEFAULT_VIEW`] unless a [`View`] elsewhere in the workspace's list
//! claims its id. Named views are the *exception list*; the default view is
//! never stored — its membership is always the derived complement. This is
//! exactly `views.js`'s `assignments` map, just grouped by view instead of
//! keyed by session: a session's list membership there is one row of this
//! module's `session_ids`.
//!
//! Consequently a workspace with no detached sessions has an empty `views`
//! entry (or none at all) and everything renders through the default view,
//! unchanged from pre-p10 behavior — the single-view case costs nothing.
//!
//! Two invariants callers must preserve (both enforced by [`prune_views`],
//! and by `WallStore::detach_focused` inline):
//!  - a session id appears in at most one [`View`]'s `session_ids`;
//!  - a [`View`] with an empty `session_ids` is removed, never left behind
//!    (spec "Detach and rejoin": "Empty non-default views SHALL be removed
//!    automatically" — the same rule covers a session dying out from under
//!    a solo view on refetch).

use std::collections::{HashMap, HashSet};

use crate::state::wall_state::WallSession;

/// The implicit view every session starts in and rejoins on detach-undo.
/// Reserved: no named view may take this name (spec: view names are
/// user-facing labels derived from a session's label, and this one is
/// already taken).
pub const DEFAULT_VIEW: &str = "main";

/// One named (i.e. non-default) view: a workspace's exception-list entry.
/// `session_ids` is never empty in stored state — see the module docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub name: String,
    pub session_ids: Vec<String>,
}

/// A workspace's named views, keyed by workspace name. The shape persisted
/// in `wall.json` and threaded through [`crate::state::persistence`].
pub type ViewsByWorkspace = HashMap<String, Vec<View>>;

/// A view's derived membership for rendering (the view strip, the group
/// frame's session count, the rail's per-view amber dot). Never constructed
/// for an empty view — see [`compute_views`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewSummary {
    pub name: String,
    /// Member session ids, in the same (salience) order as the `sessions`
    /// slice [`compute_views`] was given.
    pub session_ids: Vec<String>,
    pub needs_input_count: usize,
}

/// The view `session_id` belongs to: the named view that lists it, else
/// [`DEFAULT_VIEW`] (port of `views.js viewOf`, which defaults the same
/// way). Doesn't require `session_id` to be a real session — a stale id
/// simply matches nothing and falls through to the default.
pub fn view_of(named: &[View], session_id: &str) -> String {
    named
        .iter()
        .find(|v| v.session_ids.iter().any(|id| id == session_id))
        .map(|v| v.name.clone())
        .unwrap_or_else(|| DEFAULT_VIEW.to_owned())
}

/// Derive the ordered, non-empty view list for a workspace's sessions (port
/// of `views.js computeViews`): [`DEFAULT_VIEW`] first if it has any
/// members, then `named` views in their given (creation) order — a view
/// with no member among `sessions` simply doesn't appear, so an id in
/// `named` referencing a session that no longer exists is silently inert
/// here (pruning it out of `named` for good is [`prune_views`]'s job, run
/// separately on load/refetch).
///
/// `sessions` should already be in salience order (e.g. a
/// `WorkspaceGroup::sessions` slice) — each summary's `session_ids`
/// preserves that relative order, which is what the grid's per-view cap
/// (store.rs) and the strip's rendering both rely on.
pub fn compute_views(sessions: &[WallSession], named: &[View]) -> Vec<ViewSummary> {
    let mut order: Vec<String> = vec![DEFAULT_VIEW.to_owned()];
    for v in named {
        if !order.contains(&v.name) {
            order.push(v.name.clone());
        }
    }

    let mut buckets: HashMap<String, Vec<String>> =
        order.iter().cloned().map(|name| (name, Vec::new())).collect();
    let mut needs_input: HashSet<&str> = HashSet::new();
    for s in sessions {
        buckets.entry(view_of(named, &s.id)).or_default().push(s.id.clone());
        if s.needs_input() {
            needs_input.insert(s.id.as_str());
        }
    }

    order
        .into_iter()
        .filter_map(|name| {
            let session_ids = buckets.remove(&name)?;
            if session_ids.is_empty() {
                return None;
            }
            let needs_input_count =
                session_ids.iter().filter(|id| needs_input.contains(id.as_str())).count();
            Some(ViewSummary { name, session_ids, needs_input_count })
        })
        .collect()
}

/// A free view name derived from a session label (port of `views.js
/// deriveViewName`): `label` itself if it's neither taken nor the reserved
/// default name, else `label-2`, `label-3`, … . An empty label falls back
/// to `"view"` (bare labels shouldn't occur, but a nameless solo view would
/// be worse than a generic one).
pub fn derive_view_name(label: &str, existing_names: &[String]) -> String {
    let base = if label.is_empty() { "view" } else { label };
    let taken = |name: &str| name == DEFAULT_VIEW || existing_names.iter().any(|n| n == name);
    if !taken(base) {
        return base.to_owned();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Whether `view` currently exists for `workspace`: the default view always
/// does; a named view does iff it's present in `views` (and therefore, by
/// the module invariant, non-empty). Used to catch a `focused_view` entry
/// that pruning just orphaned (store.rs `reconcile`).
pub fn view_exists(views: &ViewsByWorkspace, workspace: &str, view: &str) -> bool {
    view == DEFAULT_VIEW
        || views.get(workspace).is_some_and(|list| list.iter().any(|v| v.name == view))
}

/// Remove ids that no longer exist from every named view, then drop any
/// view left with no members (spec "View persistence": "prune... on load
/// and on refetch"; the same rule spec "Detach and rejoin" states for a
/// session dying out from under a solo view). A workspace left with no
/// named views at all is dropped from the map so an empty entry never
/// lingers (keeps `wall.json` minimal and equality checks — the debounced-
/// save dirty check — meaningful).
pub fn prune_views(views: &mut ViewsByWorkspace, valid_ids: &HashSet<&str>) {
    for list in views.values_mut() {
        for v in list.iter_mut() {
            v.session_ids.retain(|id| valid_ids.contains(id.as_str()));
        }
        list.retain(|v| !v.session_ids.is_empty());
    }
    views.retain(|_, list| !list.is_empty());
}

#[cfg(test)]
mod tests {
    //! Parity port of `ui/test/views.test.js` (6 cases) plus prune/exists
    //! coverage for the Rust-only exception-list-of-Views storage shape.
    use super::*;

    fn session(id: &str, status: &str) -> WallSession {
        WallSession {
            id: id.to_owned(),
            workspace: "a".to_owned(),
            label: id.rsplit('/').next().unwrap_or(id).to_owned(),
            dir: None,
            status: status.to_owned(),
            since: None,
            message: None,
            branch: None,
            worktree: false,
            title: None,
        }
    }

    fn idle(id: &str) -> WallSession {
        session(id, "idle")
    }

    fn view(name: &str, ids: &[&str]) -> View {
        View { name: name.to_owned(), session_ids: ids.iter().map(|s| s.to_owned().to_owned()).collect() }
    }

    // ── computeViews parity (views.test.js) ────────────────────────────

    #[test]
    fn no_assignments_one_main_view_holding_everything() {
        let views = compute_views(&[idle("a"), idle("b")], &[]);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].name, DEFAULT_VIEW);
        assert_eq!(views[0].session_ids, ["a", "b"]);
    }

    #[test]
    fn assignments_split_into_views_empty_views_vanish() {
        // "dead" (assigned to "ghost-view") isn't a real session — matches
        // views.js's assignments-reference-a-gone-id case exactly, since
        // compute_views iterates the *session* list, never `named` directly.
        let named = [view("solo", &["c"]), view("ghost-view", &["dead"])];
        let views = compute_views(&[idle("a"), idle("c")], &named);
        let names: Vec<&str> = views.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, [DEFAULT_VIEW, "solo"]);
        assert_eq!(views[1].session_ids, ["c"]);
    }

    #[test]
    fn main_itself_vanishes_when_every_session_is_detached() {
        let named = [view("solo", &["a"])];
        let views = compute_views(&[idle("a")], &named);
        let names: Vec<&str> = views.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["solo"]);
    }

    #[test]
    fn needs_count_aggregates_per_view() {
        let named = [view("solo", &["c"])];
        let sessions = [session("a", "needs-input"), idle("b"), session("c", "needs-input")];
        let views = compute_views(&sessions, &named);
        let by_name = |n: &str| views.iter().find(|v| v.name == n).unwrap();
        assert_eq!(by_name(DEFAULT_VIEW).needs_input_count, 1);
        assert_eq!(by_name("solo").needs_input_count, 1);
    }

    #[test]
    fn view_of_defaults_to_main() {
        assert_eq!(view_of(&[], "x"), DEFAULT_VIEW);
        assert_eq!(view_of(&[view("solo", &["x"])], "x"), "solo");
    }

    #[test]
    fn derive_view_name_avoids_collisions_and_the_reserved_main_name() {
        assert_eq!(derive_view_name("test", &["main".to_owned()]), "test");
        assert_eq!(
            derive_view_name("test", &["main".to_owned(), "test".to_owned()]),
            "test-2"
        );
        assert_eq!(derive_view_name("main", &["main".to_owned()]), "main-2");
    }

    // ── Rust-only: prune_views / view_exists ───────────────────────────

    #[test]
    fn prune_views_drops_dead_ids_and_then_empty_views_and_empty_workspaces() {
        let mut views: ViewsByWorkspace = HashMap::new();
        views.insert("a".to_owned(), vec![view("solo", &["live", "dead"])]);
        views.insert("b".to_owned(), vec![view("gone", &["dead-only"])]);
        let valid: HashSet<&str> = ["live"].into_iter().collect();

        prune_views(&mut views, &valid);

        assert_eq!(views.get("a").unwrap(), &[view("solo", &["live"])]);
        assert!(!views.contains_key("b"), "an all-dead workspace is dropped entirely");
    }

    #[test]
    fn prune_views_is_a_no_op_when_nothing_is_stale() {
        let mut views: ViewsByWorkspace = HashMap::new();
        views.insert("a".to_owned(), vec![view("solo", &["live"])]);
        let before = views.clone();
        let valid: HashSet<&str> = ["live", "other"].into_iter().collect();
        prune_views(&mut views, &valid);
        assert_eq!(views, before);
    }

    #[test]
    fn view_exists_default_always_named_only_when_present() {
        let mut views: ViewsByWorkspace = HashMap::new();
        views.insert("a".to_owned(), vec![view("solo", &["x"])]);
        assert!(view_exists(&views, "a", DEFAULT_VIEW));
        assert!(view_exists(&views, "unregistered-workspace", DEFAULT_VIEW));
        assert!(view_exists(&views, "a", "solo"));
        assert!(!view_exists(&views, "a", "ghost"));
        assert!(!view_exists(&views, "b", "solo"));
    }
}
