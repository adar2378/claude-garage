//! Pure state transforms behind the store: reconciling a refetch into every
//! derived slot, and the focus helpers (moved out of `store.rs`, p22 task 1.8).

use std::collections::HashSet;

use crate::state::salience::build_groups;
use crate::state::views::{compute_views, prune_views, view_exists, view_of};
use crate::state::wall_state::{KeyLayer, WallSession, WallState};

/// Invariant: a maximized tile is always the focused one. Any transition
/// that lands focus elsewhere (focus keys, rail clicks, reconciliation
/// after the session died) exits maximize.
pub(super) fn clear_stale_maximize(mut s: WallState) -> WallState {
    if s.maximized_session_id.is_some() && s.maximized_session_id != s.focused_session_id {
        s.maximized_session_id = None;
    }
    s
}

/// Rebuild groups from `sessions` and reconcile every derived slot: views
/// pruned of dead sessions (spec tui-views "View persistence": "prune...
/// on load AND on refetch"), a dangling focused-view name reset to the
/// default, focused workspace still exists (else first group), the
/// (workspace, view)-scoped grid pruned/refilled, focused session still
/// valid, engagement dropped if its session died. Returns whether view
/// membership actually changed (pruning removed something) — the caller
/// bumps `WallStore::views_revision` on `true`, the debounced-save hook.
pub(super) fn reconcile(base: WallState, sessions: Vec<WallSession>) -> (WallState, bool) {
    let groups = build_groups(&base.workspaces, &sessions);
    // Engagement is pinned to a specific session: if reconciliation moves or
    // clears the focus (the engaged session died), drop back to garage —
    // never silently transfer engagement to another tile.
    let engaged_id = if base.layer == KeyLayer::Engaged {
        base.focused_session_id.clone()
    } else {
        None
    };
    let mut next = base;
    next.sessions = sessions;
    next.groups = groups;

    let valid_ids: HashSet<&str> = next.sessions.iter().map(|s| s.id.as_str()).collect();
    let mut views = next.views.clone();
    prune_views(&mut views, &valid_ids);
    let views_changed = views != next.views;
    next.views = views;

    // A focused-view name pruning just orphaned (its view emptied and was
    // dropped) falls back to the default — never a dangling name.
    let stale_focus: Vec<String> = next
        .focused_view
        .iter()
        .filter(|(ws, view)| !view_exists(&next.views, ws, view))
        .map(|(ws, _)| ws.clone())
        .collect();
    for ws in stale_focus {
        next.focused_view.remove(&ws);
    }
    next = refocus_vacated_views(next);

    let focused_group = next.focused_workspace.as_deref().and_then(|w| {
        next.group_by_name(Some(w))?;
        let view = next.focused_view_name(w).to_owned();
        Some(next.view_session_ids(w, &view))
    });
    let Some(group_session_ids) = focused_group else {
        // The engaged session's workspace vanished with it.
        if next.layer == KeyLayer::Engaged {
            next.layer = KeyLayer::Garage;
        }
        let first = next.groups.first().map(|g| g.name.clone());
        return (with_focused_workspace(next, first.as_deref()), views_changed);
    };

    let ids: HashSet<&str> = group_session_ids.iter().map(String::as_str).collect();
    let mut grid: Vec<String> = next
        .gridded_session_ids
        .iter()
        .filter(|id| ids.contains(id.as_str()))
        .cloned()
        .collect();
    for id in &group_session_ids {
        if grid.len() >= WallState::GRID_CAP {
            break;
        }
        if !grid.contains(id) {
            grid.push(id.clone());
        }
    }
    let mut recency: Vec<String> = next
        .grid_focus_recency
        .iter()
        .filter(|id| grid.contains(id))
        .cloned()
        .collect();
    for id in &grid {
        if !next.grid_focus_recency.contains(id) {
            recency.push(id.clone());
        }
    }

    let focused_id = match next.focused_session_id.take() {
        Some(f) if ids.contains(f.as_str()) => Some(f),
        _ => grid.first().cloned(),
    };
    next.gridded_session_ids = grid;
    next.grid_focus_recency = recency;
    next.focused_session_id = focused_id;

    // Engagement survives only while its own session is still focused and
    // live.
    if next.layer == KeyLayer::Engaged {
        let ok = next
            .session_by_id(next.focused_session_id.as_deref())
            .is_some_and(|s| s.live() && Some(s.id.as_str()) == engaged_id.as_deref());
        if !ok {
            next.layer = KeyLayer::Garage;
        }
    }
    (clear_stale_maximize(next), views_changed)
}

/// Never leave a workspace's focused view pointing at one with zero members
/// while another of its views holds sessions (the "fully-vacated default is
/// a trap after restart" bug: `focused_view` is deliberately not part of
/// `wall.json`'s persisted schema — see [`WallState::focused_view`] — so a
/// fresh process assumes every workspace's [`DEFAULT_VIEW`] until told
/// otherwise, which is wrong the moment every session has been `D`-moved
/// into named views before the restart). Runs on every reconciliation (a
/// cold `load_views` and every live refetch alike), so the same rule also
/// catches a session dying out from under the currently-focused view live,
/// not just the restart shape.
///
/// Picks the first non-empty view in the exact same display order the view
/// strip and `Tab` use ([`compute_views`]: default first when non-empty,
/// then named views in their stored/creation order). A workspace whose
/// sessions are ALL gone is left alone — nothing non-empty to redirect to,
/// so it keeps rendering the (correct, distinct) "no sessions in this
/// workspace" hint.
pub(super) fn refocus_vacated_views(mut next: WallState) -> WallState {
    let ws_sessions: Vec<(String, Vec<WallSession>)> =
        next.groups.iter().map(|g| (g.name.clone(), g.sessions.clone())).collect();
    for (name, sessions) in ws_sessions {
        let current = next.focused_view_name(&name).to_owned();
        if !next.view_session_ids(&name, &current).is_empty() {
            continue; // the focused view already has members — nothing to do.
        }
        let named = next.views.get(&name).cloned().unwrap_or_default();
        let Some(first) = compute_views(&sessions, &named).into_iter().next() else {
            continue; // the whole workspace is empty — leave as is.
        };
        if first.name != current {
            next.focused_view.insert(name, first.name);
        }
    }
    next
}

/// Switch the focused workspace: rebuild the grid as the *focused view's*
/// first [`WallState::GRID_CAP`] sessions in salience order (the workspace's
/// remembered `focused_view`, defaulting to [`DEFAULT_VIEW`] — see
/// `WallState::focused_view_name`), focus the first tile. The per-view cap
/// (spec tui-views "View partitioning": "the 6-tile cap and overflow rules
/// apply per view") falls out of scoping `view_session_ids` to that one
/// view instead of the whole workspace group.
pub(super) fn with_focused_workspace(base: WallState, name: Option<&str>) -> WallState {
    let mut next = base;
    next.focused_workspace = name.map(str::to_owned);
    let grid: Vec<String> = match name {
        Some(n) => {
            let view = next.focused_view_name(n).to_owned();
            next.view_session_ids(n, &view).into_iter().take(WallState::GRID_CAP).collect()
        }
        None => Vec::new(),
    };
    next.focused_session_id = grid.first().cloned();
    next.gridded_session_ids = grid.clone();
    next.grid_focus_recency = grid;
    clear_stale_maximize(next)
}

/// Focus `session_id`, landing it in `(workspace, view)`: switching there
/// first (a fresh salience-ordered grid, same rule as
/// [`with_focused_workspace`]) if that pair isn't already focused, then
/// making sure `session_id` itself is gridded and focused via the
/// append/LRU-swap-in rule. The one primitive behind plain session focus,
/// `d`, `Tab`, and both jump commands — see [`focus_session_state`] and the
/// `WallStore` view methods.
pub(super) fn focus_in_view(base: WallState, workspace: &str, view: &str, session_id: &str) -> WallState {
    let switching =
        base.focused_workspace.as_deref() != Some(workspace) || base.focused_view_name(workspace) != view;
    let mut next = base;
    next.focused_view.insert(workspace.to_owned(), view.to_owned());
    if switching {
        next.focused_workspace = Some(workspace.to_owned());
        let grid: Vec<String> =
            next.view_session_ids(workspace, view).into_iter().take(WallState::GRID_CAP).collect();
        next.gridded_session_ids = grid.clone();
        next.grid_focus_recency = grid;
    }

    let mut grid = next.gridded_session_ids.clone();
    let mut recency = next.grid_focus_recency.clone();
    if !grid.iter().any(|g| g == session_id) {
        if grid.len() < WallState::GRID_CAP {
            grid.push(session_id.to_owned());
        } else {
            // LRU swap-in: the least-recently-focused tile leaves; the new
            // tile takes its slot so the other five don't reshuffle.
            let lru = recency
                .iter()
                .find(|r| grid.contains(r))
                .cloned()
                .unwrap_or_else(|| grid[0].clone());
            let slot = grid.iter().position(|g| *g == lru).unwrap_or(0);
            grid[slot] = session_id.to_owned();
            recency.retain(|r| *r != lru);
        }
    }
    recency.retain(|r| r != session_id);
    recency.push(session_id.to_owned());

    next.focused_session_id = Some(session_id.to_owned());
    next.gridded_session_ids = grid;
    next.grid_focus_recency = recency;
    clear_stale_maximize(next)
}

/// Focus `id`, switching workspace and view (and swapping into the
/// (workspace, view)-scoped grid if needed). Returns `None` when the
/// session doesn't exist.
pub(super) fn focus_session_state(base: WallState, id: &str) -> Option<WallState> {
    let session = base.session_by_id(Some(id))?;
    let workspace = session.workspace.clone();
    let named = base.views.get(&workspace).cloned().unwrap_or_default();
    let view = view_of(&named, id);
    Some(focus_in_view(base, &workspace, &view, id))
}
