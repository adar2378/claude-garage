//! The wall store: the only mutable slot in the TUI (port of
//! `tui/lib/state/store.dart`). It applies events (SSE, fetches, key
//! commands) to the [`WallState`] — rendering and PTY wiring subscribe via
//! `on_change` and are pure consumers.

use std::collections::{HashMap, HashSet};

use crate::api::models::{SessionInfo, WorkspaceInfo};
use crate::state::salience::{build_groups, jump_target};
use crate::state::wall_state::{KeyLayer, OverlayKind, WallSession, WallState};

/// Garage-layer commands (spec tui-key-routing: "Garage-layer bindings" +
/// "p8.1 session lifecycle bindings" + "p8.3 workspace removal",
/// `1-9 [ ] a A n N m R x X w Enter ? q`).
/// State-affecting commands are applied by [`WallStore::dispatch`];
/// [`GarageCommand::Spawn`], [`GarageCommand::RestoreAll`],
/// [`GarageCommand::Close`], [`GarageCommand::WorkspaceRemove`] and
/// [`GarageCommand::Quit`] are effects the caller performs (daemon API
/// calls, process exit) — the store never does IO. [`GarageCommand::Engage`]
/// is declined when the focused tile is a restorable placeholder, so the
/// caller can run the restore effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GarageCommand {
    /// Zero-based index into [`WallState::groups`] (`1` → 0).
    FocusWorkspace(usize),
    /// `]` → +1, `[` → -1, cycling the focused tile through the grid.
    CycleFocus(i32),
    Jump,
    OpenQueue,
    Spawn { worktree: bool },
    Engage,
    /// `m`: toggle the focused tile full-grid (spec tui-wall "Maximized tile").
    Maximize,
    /// `R`: restore every restorable session in the focused workspace — an
    /// effect (parallel per-id `POST /api/sessions/restore` calls, like the
    /// web UI's restore-all), so the store always declines it.
    RestoreAll,
    /// `x`: armed double-press close of the focused session — an effect
    /// (`DELETE /api/sessions/<id>`), so the store always declines it.
    Close,
    /// `w`: open the add-workspace overlay.
    WorkspaceAdd,
    /// `X`: armed double-press removal of the FOCUSED workspace's
    /// registration — an effect (`DELETE /api/workspaces/<name>`,
    /// registry-only: live sessions keep running and reappear as an
    /// unregistered group), so the store always declines it.
    WorkspaceRemove,
    ToggleHelp,
    Quit,
}

/// Maps a garage-layer key to its command; `None` for unbound keys (which
/// are consumed silently — garage typing never reaches an agent). Enter
/// arrives as `"\n"` or `"\r"` depending on the host terminal.
pub fn garage_command_for(key: &str) -> Option<GarageCommand> {
    let bytes = key.as_bytes();
    if bytes.len() == 1 && (b'1'..=b'9').contains(&bytes[0]) {
        return Some(GarageCommand::FocusWorkspace((bytes[0] - b'1') as usize));
    }
    match key {
        "[" => Some(GarageCommand::CycleFocus(-1)),
        "]" => Some(GarageCommand::CycleFocus(1)),
        "a" => Some(GarageCommand::Jump),
        "A" => Some(GarageCommand::OpenQueue),
        "n" => Some(GarageCommand::Spawn { worktree: false }),
        "N" => Some(GarageCommand::Spawn { worktree: true }),
        "m" => Some(GarageCommand::Maximize),
        "R" => Some(GarageCommand::RestoreAll),
        "x" => Some(GarageCommand::Close),
        "w" => Some(GarageCommand::WorkspaceAdd),
        "X" => Some(GarageCommand::WorkspaceRemove),
        "\n" | "\r" => Some(GarageCommand::Engage),
        "?" => Some(GarageCommand::ToggleHelp),
        "q" => Some(GarageCommand::Quit),
        _ => None,
    }
}

type OnChange = Box<dyn FnMut(&WallState) + Send>;

#[derive(Default)]
pub struct WallStore {
    state: WallState,
    /// Bumped on every state change — lets callers (and tests, standing in
    /// for Dart's `same(before)` identity checks) detect "no change".
    version: u64,
    /// Called after every state change; render wiring hangs off this.
    pub on_change: Option<OnChange>,
}

impl WallStore {
    pub fn new() -> WallStore {
        WallStore::default()
    }

    pub fn state(&self) -> &WallState {
        &self.state
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    fn set(&mut self, next: WallState) {
        self.state = next;
        self.version += 1;
        if let Some(cb) = self.on_change.as_mut() {
            cb(&self.state);
        }
    }

    // ── Fetch / SSE events ────────────────────────────────────────────────

    pub fn workspaces_fetched(&mut self, workspaces: Vec<WorkspaceInfo>) {
        // Re-derive worktree flags: the flag compares a session's dir against
        // its workspace's registered dir, which may just have changed.
        let dirs: HashMap<&str, Option<&str>> = workspaces
            .iter()
            .map(|w| (w.name.as_str(), w.dir.as_deref()))
            .collect();
        let sessions: Vec<WallSession> = self
            .state
            .sessions
            .iter()
            .map(|s| {
                let registered_dir = dirs.get(s.workspace.as_str()).copied().flatten();
                WallSession {
                    worktree: s.live()
                        && s.dir.is_some()
                        && registered_dir.is_some()
                        && s.dir.as_deref() != registered_dir,
                    ..s.clone()
                }
            })
            .collect();
        let mut base = self.state.clone();
        base.workspaces = workspaces;
        let next = reconcile(base, sessions);
        self.set(next);
    }

    pub fn sessions_fetched(&mut self, infos: Vec<SessionInfo>) {
        let dirs: HashMap<&str, Option<&str>> = self
            .state
            .workspaces
            .iter()
            .map(|w| (w.name.as_str(), w.dir.as_deref()))
            .collect();
        let sessions: Vec<WallSession> = infos
            .iter()
            .map(|info| {
                WallSession::from_info(info, dirs.get(info.workspace.as_str()).copied().flatten())
            })
            .collect();
        let next = reconcile(self.state.clone(), sessions);
        self.set(next);
    }

    /// SSE `status` event `{id, status, since}`. The event carries no
    /// message; the daemon clears the message on any transition away from
    /// needs-input, so mirror that here — a needs-input transition's message
    /// text arrives with the next sessions refetch.
    pub fn status_changed(&mut self, id: &str, status: &str, since: Option<i64>) {
        if !self.state.sessions.iter().any(|s| s.id == id) {
            return; // Unknown id — the sessions refetch will bring it.
        }
        let sessions: Vec<WallSession> = self
            .state
            .sessions
            .iter()
            .map(|s| {
                if s.id == id {
                    let message = if status == "needs-input" {
                        s.message.clone()
                    } else {
                        None
                    };
                    s.copy_with(Some(status), since, message)
                } else {
                    s.clone()
                }
            })
            .collect();
        let next = reconcile(self.state.clone(), sessions);
        self.set(next);
    }

    // ── Focus ─────────────────────────────────────────────────────────────

    /// Focus the `index`-th group in salience order (the `1`–`9` bindings).
    pub fn focus_workspace(&mut self, index: usize) {
        let Some(group) = self.state.groups.get(index) else {
            return;
        };
        let name = group.name.clone();
        let next = with_focused_workspace(self.state.clone(), Some(&name));
        self.set(next);
    }

    /// Focus a workspace by name (the add-workspace flow lands on the group
    /// it just created — an index would race the salience reorder).
    pub fn focus_workspace_named(&mut self, name: &str) {
        if self.state.group_by_name(Some(name)).is_none() {
            return;
        }
        let next = with_focused_workspace(self.state.clone(), Some(name));
        self.set(next);
    }

    /// Focus a session by id, switching workspace and swapping the session
    /// into the grid (LRU eviction) when needed.
    pub fn focus_session(&mut self, id: &str) {
        if let Some(next) = focus_session_state(self.state.clone(), id) {
            self.set(next);
        }
    }

    /// Cycle the focused tile through the grid (`[` / `]`).
    pub fn cycle_focus(&mut self, delta: i32) {
        let grid = self.state.gridded_session_ids.clone();
        if grid.is_empty() {
            return;
        }
        let focused = self.state.focused_session_id.clone().unwrap_or_default();
        let from = grid.iter().position(|id| *id == focused).unwrap_or(0);
        let next = (from as i32 + delta).rem_euclid(grid.len() as i32) as usize;
        let target = grid[next].clone();
        self.focus_session(&target);
    }

    /// Toggle the focused tile full-grid (`m`, spec tui-wall "Maximized
    /// tile"). Only a gridded tile can maximize; toggling the maximized tile
    /// (or focusing another session — see [`clear_stale_maximize`]) restores
    /// the normal grid.
    pub fn toggle_maximize(&mut self) {
        if self.state.layer != KeyLayer::Garage {
            return;
        }
        let Some(id) = self.state.focused_session_id.clone() else {
            return;
        };
        if self.state.maximized_session_id.as_deref() == Some(id.as_str()) {
            let mut next = self.state.clone();
            next.maximized_session_id = None;
            self.set(next);
        } else if self.state.gridded_session_ids.contains(&id) {
            let mut next = self.state.clone();
            next.maximized_session_id = Some(id);
            self.set(next);
        }
    }

    // ── Layers ────────────────────────────────────────────────────────────

    /// Engage the focused tile. Requires the garage layer and a focused live
    /// session — a restorable placeholder (or no focus) cannot be engaged.
    /// Returns whether the engage happened, so the caller can offer the
    /// restore effect for a declined restorable tile.
    pub fn engage(&mut self) -> bool {
        if self.state.layer != KeyLayer::Garage {
            return false;
        }
        let live = self
            .state
            .session_by_id(self.state.focused_session_id.as_deref())
            .is_some_and(WallSession::live);
        if !live {
            return false;
        }
        let mut next = self.state.clone();
        next.layer = KeyLayer::Engaged;
        self.set(next);
        true
    }

    /// Return from engaged to garage (the Ctrl+G/Ctrl+Q chord).
    pub fn disengage(&mut self) {
        if self.state.layer != KeyLayer::Engaged {
            return;
        }
        let mut next = self.state.clone();
        next.layer = KeyLayer::Garage;
        self.set(next);
    }

    /// Open an overlay from the garage layer. Overlays never stack: while
    /// one is open (or a tile is engaged) this is a no-op.
    pub fn open_overlay(&mut self, kind: OverlayKind) {
        if self.state.layer != KeyLayer::Garage {
            return;
        }
        let mut next = self.state.clone();
        next.layer = KeyLayer::Overlay;
        next.overlay = Some(kind);
        self.set(next);
    }

    pub fn close_overlay(&mut self) {
        if self.state.layer != KeyLayer::Overlay {
            return;
        }
        let mut next = self.state.clone();
        next.layer = KeyLayer::Garage;
        next.overlay = None;
        self.set(next);
    }

    // ── Triage ────────────────────────────────────────────────────────────

    /// The `a` jump (spec tui-triage): focus the longest-waiting needs-input
    /// session across all workspaces — switching workspace, swapping the
    /// tile in if it was overflow — and land engaged. Returns false (state
    /// untouched) when nothing is blocked; the caller renders the strip
    /// notice.
    pub fn jump_to_longest_waiting(&mut self) -> bool {
        if self.state.layer != KeyLayer::Garage {
            return false;
        }
        let Some(target_id) = jump_target(&self.state.sessions).map(|s| s.id.clone()) else {
            return false;
        };
        let Some(mut focused) = focus_session_state(self.state.clone(), &target_id) else {
            return false;
        };
        // needs-input implies live, so the engage precondition holds.
        focused.layer = KeyLayer::Engaged;
        self.set(focused);
        true
    }

    /// Jump-and-engage a specific blocked session (Enter in the triage
    /// queue, spec tui-triage: "Jump from queue lands engaged") — same
    /// landing as [`WallStore::jump_to_longest_waiting`] but for a chosen
    /// id. Returns false (state untouched) when the session is gone or no
    /// longer needs-input, so a stale queue row can never engage the wrong
    /// tile.
    pub fn jump_to_session(&mut self, id: &str) -> bool {
        if self.state.layer != KeyLayer::Garage {
            return false;
        }
        let ok = self
            .state
            .session_by_id(Some(id))
            .is_some_and(WallSession::needs_input);
        if !ok {
            return false;
        }
        let Some(mut focused) = focus_session_state(self.state.clone(), id) else {
            return false;
        };
        // needs-input implies live, so the engage precondition holds.
        focused.layer = KeyLayer::Engaged;
        self.set(focused);
        true
    }

    // ── Command dispatch ──────────────────────────────────────────────────

    /// Apply a garage-layer command to the state. Returns true when the
    /// store handled it; `Spawn`, `RestoreAll`, `Close`, `WorkspaceRemove`
    /// and `Quit` always return false — they are the caller's effects.
    /// `Engage` returns false when the focused tile cannot engage (a
    /// restorable placeholder), so the caller can run the restore effect.
    /// `ToggleHelp` also closes an open help overlay, so `?` toggles.
    pub fn dispatch(&mut self, command: GarageCommand) -> bool {
        match command {
            GarageCommand::ToggleHelp => {
                if self.state.layer == KeyLayer::Overlay {
                    if self.state.overlay != Some(OverlayKind::Help) {
                        return false;
                    }
                    self.close_overlay();
                    return true;
                }
                if self.state.layer != KeyLayer::Garage {
                    return false;
                }
                self.open_overlay(OverlayKind::Help);
                true
            }
            GarageCommand::Spawn { .. }
            | GarageCommand::RestoreAll
            | GarageCommand::Close
            | GarageCommand::WorkspaceRemove
            | GarageCommand::Quit => false,
            _ if self.state.layer != KeyLayer::Garage => false,
            GarageCommand::FocusWorkspace(index) => {
                self.focus_workspace(index);
                true
            }
            GarageCommand::CycleFocus(delta) => {
                self.cycle_focus(delta);
                true
            }
            GarageCommand::Jump => self.jump_to_longest_waiting(),
            GarageCommand::OpenQueue => {
                self.open_overlay(OverlayKind::TriageQueue);
                true
            }
            GarageCommand::Maximize => {
                self.toggle_maximize();
                true
            }
            GarageCommand::WorkspaceAdd => {
                self.open_overlay(OverlayKind::WorkspaceAdd);
                true
            }
            GarageCommand::Engage => self.engage(),
        }
    }
}

// ── Internals (pure state transforms) ─────────────────────────────────────

/// Invariant: a maximized tile is always the focused one. Any transition
/// that lands focus elsewhere (focus keys, rail clicks, reconciliation
/// after the session died) exits maximize.
fn clear_stale_maximize(mut s: WallState) -> WallState {
    if s.maximized_session_id.is_some() && s.maximized_session_id != s.focused_session_id {
        s.maximized_session_id = None;
    }
    s
}

/// Rebuild groups from `sessions` and reconcile every derived slot: focused
/// workspace still exists (else first group), grid pruned/refilled, focused
/// session still valid, engagement dropped if its session died.
fn reconcile(base: WallState, sessions: Vec<WallSession>) -> WallState {
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

    let focused_group = next
        .group_by_name(next.focused_workspace.as_deref())
        .map(|g| g.sessions.iter().map(|s| s.id.clone()).collect::<Vec<_>>());
    let Some(group_session_ids) = focused_group else {
        // The engaged session's workspace vanished with it.
        if next.layer == KeyLayer::Engaged {
            next.layer = KeyLayer::Garage;
        }
        let first = next.groups.first().map(|g| g.name.clone());
        return with_focused_workspace(next, first.as_deref());
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
    clear_stale_maximize(next)
}

/// Switch the focused workspace: rebuild the grid as the group's first
/// [`WallState::GRID_CAP`] sessions in salience order, focus the first tile.
fn with_focused_workspace(base: WallState, name: Option<&str>) -> WallState {
    let grid: Vec<String> = base
        .group_by_name(name)
        .map(|g| {
            g.sessions
                .iter()
                .take(WallState::GRID_CAP)
                .map(|s| s.id.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut next = base;
    next.focused_workspace = name.map(str::to_owned);
    next.focused_session_id = grid.first().cloned();
    next.gridded_session_ids = grid.clone();
    next.grid_focus_recency = grid;
    clear_stale_maximize(next)
}

/// Focus `id`, switching workspace and swapping into the grid if needed.
/// Returns `None` when the session doesn't exist.
fn focus_session_state(base: WallState, id: &str) -> Option<WallState> {
    let workspace = base.session_by_id(Some(id))?.workspace.clone();

    let mut next = base;
    if next.focused_workspace.as_deref() != Some(workspace.as_str()) {
        next = with_focused_workspace(next, Some(&workspace));
    }

    let mut grid = next.gridded_session_ids.clone();
    let mut recency = next.grid_focus_recency.clone();
    if !grid.iter().any(|g| g == id) {
        if grid.len() < WallState::GRID_CAP {
            grid.push(id.to_owned());
        } else {
            // LRU swap-in: the least-recently-focused tile leaves; the new
            // tile takes its slot so the other five don't reshuffle.
            let lru = recency
                .iter()
                .find(|r| grid.contains(r))
                .cloned()
                .unwrap_or_else(|| grid[0].clone());
            let slot = grid.iter().position(|g| *g == lru).unwrap_or(0);
            grid[slot] = id.to_owned();
            recency.retain(|r| *r != lru);
        }
    }
    recency.retain(|r| r != id);
    recency.push(id.to_owned());

    next.focused_session_id = Some(id.to_owned());
    next.gridded_session_ids = grid;
    next.grid_focus_recency = recency;
    Some(clear_stale_maximize(next))
}

#[cfg(test)]
mod tests {
    //! Port of `tui/test/wall_store_test.dart`: layer transitions, the 6-cap
    //! grid with LRU swap-in, the a-jump selector, garage command mapping,
    //! and SSE-event application. Specs: tui-key-routing (layers, garage
    //! bindings), tui-wall (grid cap), tui-triage (a jump).
    use super::*;
    use crate::state::salience::restorable_session_ids;

    fn ws(name: &str) -> WorkspaceInfo {
        WorkspaceInfo {
            name: name.to_owned(),
            dir: Some(format!("/repos/{name}")),
            branch: None,
        }
    }

    fn si(workspace: &str, label: &str) -> SessionInfo {
        SessionInfo {
            id: format!("garage/{workspace}/{label}"),
            workspace: workspace.to_owned(),
            label: label.to_owned(),
            dir: Some(format!("/repos/{workspace}")),
            attached: false,
            status: "working".to_owned(),
            since: Some(1000),
            message: None,
            branch: None,
            restorable: false,
        }
    }

    fn si_status(workspace: &str, label: &str, status: &str, since: Option<i64>) -> SessionInfo {
        SessionInfo {
            status: status.to_owned(),
            since,
            ..si(workspace, label)
        }
    }

    fn si_restorable(workspace: &str, label: &str) -> SessionInfo {
        SessionInfo {
            status: "restorable".to_owned(),
            since: None,
            restorable: true,
            ..si(workspace, label)
        }
    }

    fn id(workspace: &str, label: &str) -> String {
        format!("garage/{workspace}/{label}")
    }

    fn store_with(workspaces: Vec<WorkspaceInfo>, sessions: Vec<SessionInfo>) -> WallStore {
        let mut store = WallStore::new();
        store.workspaces_fetched(workspaces);
        store.sessions_fetched(sessions);
        store
    }

    // ── fetch reconciliation ────────────────────────────────────────────

    #[test]
    fn first_fetch_focuses_the_first_group_and_grids_its_sessions() {
        let store = store_with(
            vec![ws("a"), ws("b")],
            vec![si("a", "one"), si("a", "two"), si("b", "other")],
        );
        assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
        assert_eq!(store.state().focused_session_id, Some(id("a", "one")));
        assert_eq!(
            store.state().gridded_session_ids,
            [id("a", "one"), id("a", "two")]
        );
        assert_eq!(store.state().layer, KeyLayer::Garage);
        assert_eq!(store.state().keys_target_chip(), "keys → garage");
    }

    #[test]
    fn worktree_flag_derives_from_dir_differing_from_the_registered_dir() {
        let wt = SessionInfo {
            dir: Some("/repos/.garage-worktrees/a-wt".to_owned()),
            ..si("a", "wt")
        };
        let store = store_with(vec![ws("a")], vec![si("a", "plain"), wt]);
        let plain = store.state().session_by_id(Some(&id("a", "plain"))).unwrap();
        assert!(!plain.worktree);
        let wt = store.state().session_by_id(Some(&id("a", "wt"))).unwrap();
        assert!(wt.worktree);
    }

    #[test]
    fn a_vanished_focused_workspace_falls_back_to_the_first_group() {
        let mut store = store_with(vec![ws("a"), ws("b")], vec![si("a", "one")]);
        store.workspaces_fetched(vec![ws("b")]);
        store.sessions_fetched(vec![si("b", "other")]);
        assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
        assert_eq!(store.state().focused_session_id, Some(id("b", "other")));
    }

    #[test]
    fn p8_3_x_remove_transition_registry_only_removal_resurfaces_live_sessions_unregistered() {
        // Before: `a` is a registered workspace with two live sessions.
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![si("a", "one"), si("a", "two"), si("b", "other")],
        );
        assert!(store.state().group_by_name(Some("a")).unwrap().registered);

        // DELETE /api/workspaces/a (registry-only) → the refetch lists no
        // workspace `a`, but its tmux sessions are still live and listed.
        store.workspaces_fetched(vec![ws("b")]);
        store.sessions_fetched(vec![si("a", "one"), si("a", "two"), si("b", "other")]);

        let group = store
            .state()
            .group_by_name(Some("a"))
            .expect("live sessions must never become invisible");
        assert!(
            !group.registered,
            "the group is now synthesized from tmux discovery"
        );
        let ids: Vec<&str> = group.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, [id("a", "one"), id("a", "two")]);
        assert!(store
            .state()
            .session_by_id(Some(&id("a", "one")))
            .unwrap()
            .live());
        // Registered groups keep registry order first; the synthesized group
        // trails them (salience build_groups insertion order).
        let names: Vec<&str> = store.state().groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["b", "a"]);
    }

    // ── layer transitions ───────────────────────────────────────────────

    #[test]
    fn engage_requires_a_focused_live_session_empty_wall_is_a_no_op() {
        let mut store = store_with(vec![ws("a")], vec![]);
        store.engage();
        assert_eq!(store.state().layer, KeyLayer::Garage);
    }

    #[test]
    fn engage_on_a_restorable_placeholder_is_a_no_op() {
        let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "dead")]);
        assert_eq!(store.state().focused_session_id, Some(id("a", "dead")));
        store.engage();
        assert_eq!(store.state().layer, KeyLayer::Garage);
    }

    #[test]
    fn engage_on_a_live_session_flips_the_layer_and_the_chip_in_the_same_state_change() {
        let mut store = store_with(vec![ws("apexlabs")], vec![si("apexlabs", "api-fix")]);
        store.engage();
        assert_eq!(store.state().layer, KeyLayer::Engaged);
        assert_eq!(store.state().keys_target_chip(), "keys → apexlabs/api-fix");
    }

    #[test]
    fn disengage_returns_to_garage_disengaging_from_garage_is_a_no_op() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.disengage();
        assert_eq!(store.state().layer, KeyLayer::Garage);
        store.engage();
        store.disengage();
        assert_eq!(store.state().layer, KeyLayer::Garage);
        assert_eq!(store.state().keys_target_chip(), "keys → garage");
    }

    #[test]
    fn overlays_never_stack_opening_a_second_overlay_is_a_no_op() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::Help);
        assert_eq!(store.state().overlay, Some(OverlayKind::Help));
        store.open_overlay(OverlayKind::TriageQueue);
        assert_eq!(store.state().overlay, Some(OverlayKind::Help));
        assert_eq!(store.state().layer, KeyLayer::Overlay);
    }

    #[test]
    fn an_overlay_cannot_open_while_engaged() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.engage();
        store.open_overlay(OverlayKind::TriageQueue);
        assert_eq!(store.state().layer, KeyLayer::Engaged);
        assert_eq!(store.state().overlay, None);
    }

    #[test]
    fn close_overlay_returns_to_garage_and_clears_the_overlay_slot() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.open_overlay(OverlayKind::TriageQueue);
        assert_eq!(store.state().keys_target_chip(), "keys → queue");
        store.close_overlay();
        assert_eq!(store.state().layer, KeyLayer::Garage);
        assert_eq!(store.state().overlay, None);
    }

    #[test]
    fn engagement_is_dropped_when_the_engaged_session_leaves_the_listing() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.engage();
        assert_eq!(store.state().layer, KeyLayer::Engaged);
        store.sessions_fetched(vec![si("a", "two")]);
        assert_eq!(store.state().layer, KeyLayer::Garage);
        assert_eq!(store.state().focused_session_id, Some(id("a", "two")));
    }

    // ── grid: 6-cap with LRU swap-in ────────────────────────────────────

    fn seven() -> Vec<SessionInfo> {
        (1..=7).map(|i| si("a", &format!("s{i}"))).collect()
    }

    #[test]
    fn a_seventh_session_stays_off_the_grid_rail_only() {
        let store = store_with(vec![ws("a")], seven());
        assert_eq!(store.state().gridded_session_ids.len(), 6);
        assert!(!store.state().gridded_session_ids.contains(&id("a", "s7")));
    }

    #[test]
    fn focusing_the_overflow_session_swaps_it_into_the_lru_slot_other_tiles_keep_positions() {
        let mut store = store_with(vec![ws("a")], seven());
        // Touch every tile except s1, making s1 the least-recently-focused.
        for i in 2..=6 {
            store.focus_session(&id("a", &format!("s{i}")));
        }
        store.focus_session(&id("a", "s7"));
        // s7 took s1's slot (slot 0) — slot-stable swap-in.
        assert_eq!(
            store.state().gridded_session_ids,
            [
                id("a", "s7"),
                id("a", "s2"),
                id("a", "s3"),
                id("a", "s4"),
                id("a", "s5"),
                id("a", "s6"),
            ]
        );
        assert_eq!(store.state().focused_session_id, Some(id("a", "s7")));
    }

    #[test]
    fn lru_follows_focus_recency_not_grid_order() {
        let mut store = store_with(vec![ws("a")], seven());
        store.focus_session(&id("a", "s1")); // s1 is now most-recent; s2 is LRU
        store.focus_session(&id("a", "s7"));
        assert!(store.state().gridded_session_ids.contains(&id("a", "s1")));
        assert!(!store.state().gridded_session_ids.contains(&id("a", "s2")));
        let pos = store
            .state()
            .gridded_session_ids
            .iter()
            .position(|s| *s == id("a", "s7"));
        assert_eq!(pos, Some(1), "s2's old slot");
    }

    #[test]
    fn below_the_cap_focusing_an_ungridded_session_appends_it() {
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![si("a", "one"), si("b", "other")],
        );
        // Cross-workspace focus switches workspace and grids the session.
        store.focus_session(&id("b", "other"));
        assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
        assert_eq!(store.state().gridded_session_ids, [id("b", "other")]);
    }

    #[test]
    fn cycle_focus_wraps_in_both_directions() {
        let mut store = store_with(
            vec![ws("a")],
            vec![si("a", "s1"), si("a", "s2"), si("a", "s3")],
        );
        store.cycle_focus(-1);
        assert_eq!(store.state().focused_session_id, Some(id("a", "s3")));
        store.cycle_focus(1);
        assert_eq!(store.state().focused_session_id, Some(id("a", "s1")));
        store.cycle_focus(1);
        assert_eq!(store.state().focused_session_id, Some(id("a", "s2")));
    }

    #[test]
    fn a_gone_gridded_session_is_pruned_and_the_grid_refills_from_the_rail_overflow() {
        let mut store = store_with(vec![ws("a")], seven());
        store.sessions_fetched(seven().into_iter().filter(|s| s.label != "s3").collect());
        assert_eq!(store.state().gridded_session_ids.len(), 6);
        assert!(store.state().gridded_session_ids.contains(&id("a", "s7")));
        assert!(!store.state().gridded_session_ids.contains(&id("a", "s3")));
    }

    // ── a-jump ──────────────────────────────────────────────────────────

    #[test]
    fn jumps_to_the_longest_waiting_blocked_session_across_workspaces_and_lands_engaged() {
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![
                si("a", "one"),
                si_status("a", "young", "needs-input", Some(5000)),
                si_status("b", "old", "needs-input", Some(100)),
            ],
        );
        assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
        assert!(store.jump_to_longest_waiting());
        assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
        assert_eq!(store.state().focused_session_id, Some(id("b", "old")));
        assert_eq!(store.state().layer, KeyLayer::Engaged);
        assert_eq!(store.state().keys_target_chip(), "keys → b/old");
    }

    #[test]
    fn an_overflow_blocked_session_is_swapped_into_the_grid_by_the_jump() {
        // 7 blocked sessions: salience keeps listing order, so the grid holds
        // s1..s6 and the oldest-waiting s7 is overflow until the jump.
        let sessions: Vec<SessionInfo> = (1..=7)
            .map(|i| si_status("a", &format!("s{i}"), "needs-input", Some(800 - i)))
            .collect();
        let mut store = store_with(vec![ws("a")], sessions);
        assert!(!store.state().gridded_session_ids.contains(&id("a", "s7")));
        assert!(store.jump_to_longest_waiting());
        assert!(store.state().gridded_session_ids.contains(&id("a", "s7")));
        assert_eq!(store.state().focused_session_id, Some(id("a", "s7")));
        assert_eq!(store.state().layer, KeyLayer::Engaged);
    }

    #[test]
    fn with_nothing_blocked_the_jump_is_a_no_op_returning_false() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let before = store.version();
        assert!(!store.jump_to_longest_waiting());
        assert_eq!(store.version(), before, "state untouched");
        assert_eq!(store.state().layer, KeyLayer::Garage);
    }

    #[test]
    fn the_jump_only_fires_from_the_garage_layer() {
        let mut store = store_with(
            vec![ws("a")],
            vec![si("a", "one"), si_status("a", "blocked", "needs-input", Some(1000))],
        );
        store.focus_session(&id("a", "one"));
        store.engage();
        assert!(!store.jump_to_longest_waiting());
        assert_eq!(store.state().focused_session_id, Some(id("a", "one")));
        assert_eq!(store.state().layer, KeyLayer::Engaged);
    }

    // ── garage command mapping ──────────────────────────────────────────

    #[test]
    fn maps_every_bound_key() {
        assert_eq!(
            garage_command_for("1"),
            Some(GarageCommand::FocusWorkspace(0))
        );
        assert_eq!(
            garage_command_for("9"),
            Some(GarageCommand::FocusWorkspace(8))
        );
        assert_eq!(garage_command_for("["), Some(GarageCommand::CycleFocus(-1)));
        assert_eq!(garage_command_for("]"), Some(GarageCommand::CycleFocus(1)));
        assert_eq!(garage_command_for("a"), Some(GarageCommand::Jump));
        assert_eq!(garage_command_for("A"), Some(GarageCommand::OpenQueue));
        assert_eq!(
            garage_command_for("n"),
            Some(GarageCommand::Spawn { worktree: false })
        );
        assert_eq!(
            garage_command_for("N"),
            Some(GarageCommand::Spawn { worktree: true })
        );
        assert_eq!(garage_command_for("\n"), Some(GarageCommand::Engage));
        assert_eq!(garage_command_for("\r"), Some(GarageCommand::Engage));
        assert_eq!(garage_command_for("?"), Some(GarageCommand::ToggleHelp));
        assert_eq!(garage_command_for("q"), Some(GarageCommand::Quit));
    }

    #[test]
    fn unbound_keys_map_to_nothing_garage_typing_never_reaches_an_agent() {
        assert_eq!(garage_command_for("z"), None);
        assert_eq!(garage_command_for("0"), None);
        assert_eq!(garage_command_for(" "), None);
    }

    #[test]
    fn p8_1_lifecycle_keys_map_to_their_commands() {
        assert_eq!(garage_command_for("m"), Some(GarageCommand::Maximize));
        assert_eq!(garage_command_for("R"), Some(GarageCommand::RestoreAll));
        assert_eq!(garage_command_for("x"), Some(GarageCommand::Close));
        assert_eq!(garage_command_for("w"), Some(GarageCommand::WorkspaceAdd));
    }

    #[test]
    fn p8_3_shift_x_maps_to_workspace_removal_an_effect_the_store_declines() {
        assert_eq!(
            garage_command_for("X"),
            Some(GarageCommand::WorkspaceRemove)
        );
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert!(!store.dispatch(GarageCommand::WorkspaceRemove));
        assert_eq!(
            store.state().layer,
            KeyLayer::Garage,
            "declining must leave state untouched"
        );
    }

    #[test]
    fn digits_focus_workspaces_in_salience_order() {
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![si("a", "one"), si_status("b", "blocked", "needs-input", Some(1000))],
        );
        // b is blocked, so it holds rail position 1.
        assert!(store.dispatch(GarageCommand::FocusWorkspace(0)));
        assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
        assert!(store.dispatch(GarageCommand::FocusWorkspace(1)));
        assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
        // Out-of-range digit: handled (consumed) but state unchanged.
        store.dispatch(GarageCommand::FocusWorkspace(8));
        assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
    }

    #[test]
    fn question_mark_toggles_the_help_overlay() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert!(store.dispatch(GarageCommand::ToggleHelp));
        assert_eq!(store.state().overlay, Some(OverlayKind::Help));
        assert!(store.dispatch(GarageCommand::ToggleHelp));
        assert_eq!(store.state().layer, KeyLayer::Garage);
        assert_eq!(store.state().overlay, None);
    }

    #[test]
    fn spawn_and_quit_are_caller_effects_not_store_transitions() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert!(!store.dispatch(GarageCommand::Spawn { worktree: false }));
        assert!(!store.dispatch(GarageCommand::Quit));
        assert_eq!(store.state().layer, KeyLayer::Garage);
    }

    #[test]
    fn state_commands_are_inert_outside_the_garage_layer() {
        let mut store = store_with(vec![ws("a"), ws("b")], vec![si("a", "one")]);
        store.engage();
        assert!(!store.dispatch(GarageCommand::FocusWorkspace(1)));
        assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
        assert!(!store.dispatch(GarageCommand::Jump));
    }

    // ── status events ───────────────────────────────────────────────────

    #[test]
    fn status_changed_reorders_salience_and_stamps_the_new_since() {
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![si("a", "one"), si("b", "other")],
        );
        store.status_changed(&id("b", "other"), "needs-input", Some(4242));
        assert_eq!(store.state().groups[0].name, "b");
        let s = store.state().session_by_id(Some(&id("b", "other"))).unwrap();
        assert_eq!(s.status, "needs-input");
        assert_eq!(s.since, Some(4242));
    }

    #[test]
    fn a_transition_away_from_needs_input_clears_the_message_mirrors_the_daemon_store() {
        let blocked = SessionInfo {
            message: Some("Claude needs your permission".to_owned()),
            ..si_status("a", "one", "needs-input", Some(1000))
        };
        let mut store = store_with(vec![ws("a")], vec![blocked]);
        assert_eq!(
            store
                .state()
                .session_by_id(Some(&id("a", "one")))
                .unwrap()
                .message
                .as_deref(),
            Some("Claude needs your permission")
        );
        store.status_changed(&id("a", "one"), "working", Some(9000));
        assert_eq!(
            store
                .state()
                .session_by_id(Some(&id("a", "one")))
                .unwrap()
                .message,
            None
        );
    }

    #[test]
    fn status_changed_for_an_unknown_id_is_ignored_until_the_refetch() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let before = store.version();
        store.status_changed("garage/a/ghost", "needs-input", Some(1));
        assert_eq!(store.version(), before, "state untouched");
    }

    // ── maximize (p8.1, spec tui-wall "Maximized tile") ─────────────────

    #[test]
    fn m_toggles_the_focused_tile_full_grid_and_back() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        assert_eq!(store.state().maximized_session_id, None);
        store.dispatch(GarageCommand::Maximize);
        assert_eq!(store.state().maximized_session_id, Some(id("a", "one")));
        store.dispatch(GarageCommand::Maximize);
        assert_eq!(store.state().maximized_session_id, None);
    }

    #[test]
    fn focusing_another_session_exits_maximize() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.toggle_maximize();
        assert_eq!(store.state().maximized_session_id, Some(id("a", "one")));
        store.focus_session(&id("a", "two"));
        assert_eq!(store.state().maximized_session_id, None);
        assert_eq!(store.state().focused_session_id, Some(id("a", "two")));
    }

    #[test]
    fn cycling_focus_exits_maximize() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.toggle_maximize();
        store.cycle_focus(1);
        assert_eq!(store.state().maximized_session_id, None);
    }

    #[test]
    fn switching_workspace_exits_maximize() {
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![si("a", "one"), si("b", "other")],
        );
        store.toggle_maximize();
        store.focus_workspace(1);
        assert_eq!(store.state().maximized_session_id, None);
    }

    #[test]
    fn re_focusing_the_maximized_session_keeps_it_maximized() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.toggle_maximize();
        store.focus_session(&id("a", "one"));
        assert_eq!(store.state().maximized_session_id, Some(id("a", "one")));
    }

    #[test]
    fn engagement_keeps_maximize_maximize_plus_engage_is_the_workflow() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.toggle_maximize();
        store.engage();
        assert_eq!(store.state().layer, KeyLayer::Engaged);
        assert_eq!(store.state().maximized_session_id, Some(id("a", "one")));
        // m is a garage-layer binding: while engaged the toggle is inert.
        store.toggle_maximize();
        assert_eq!(store.state().maximized_session_id, Some(id("a", "one")));
    }

    #[test]
    fn reconciliation_clears_maximize_when_the_session_dies() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.toggle_maximize();
        store.sessions_fetched(vec![si("a", "two")]);
        assert_eq!(store.state().maximized_session_id, None);
        assert_eq!(store.state().focused_session_id, Some(id("a", "two")));
    }

    #[test]
    fn maximize_with_no_focus_empty_wall_is_a_no_op() {
        let mut store = store_with(vec![ws("a")], vec![]);
        store.toggle_maximize();
        assert_eq!(store.state().maximized_session_id, None);
    }

    // ── restore / close / workspace-add commands (p8.1) ─────────────────

    #[test]
    fn engage_returns_false_for_a_restorable_placeholder_caller_restores_instead() {
        let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "gone")]);
        assert!(!store.dispatch(GarageCommand::Engage));
        assert_eq!(
            store.state().layer,
            KeyLayer::Garage,
            "engage must still require a live session"
        );
    }

    #[test]
    fn engage_still_works_and_returns_true_for_a_live_session() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert!(store.dispatch(GarageCommand::Engage));
        assert_eq!(store.state().layer, KeyLayer::Engaged);
    }

    #[test]
    fn r_and_x_are_effects_the_store_always_declines_them() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert!(!store.dispatch(GarageCommand::RestoreAll));
        assert!(!store.dispatch(GarageCommand::Close));
        assert_eq!(store.state().layer, KeyLayer::Garage);
    }

    #[test]
    fn w_opens_the_add_workspace_overlay_overlays_never_stack() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert!(store.dispatch(GarageCommand::WorkspaceAdd));
        assert_eq!(store.state().layer, KeyLayer::Overlay);
        assert_eq!(store.state().overlay, Some(OverlayKind::WorkspaceAdd));
        assert_eq!(store.state().keys_target_chip(), "keys → add workspace");
        assert!(!store.dispatch(GarageCommand::WorkspaceAdd));
        store.close_overlay();
        assert_eq!(store.state().layer, KeyLayer::Garage);
    }

    #[test]
    fn focus_workspace_named_lands_on_the_named_group() {
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![si("a", "one"), si("b", "other")],
        );
        store.focus_workspace_named("b");
        assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
        assert_eq!(store.state().focused_session_id, Some(id("b", "other")));
        store.focus_workspace_named("ghost"); // unknown name is a no-op
        assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
    }

    #[test]
    fn restorable_session_ids_selects_only_restorable_sessions_of_the_workspace_in_order() {
        let store = store_with(
            vec![ws("a"), ws("b")],
            vec![
                si("a", "live"),
                si_restorable("a", "r1"),
                si_restorable("b", "r-other"),
                si_restorable("a", "r2"),
            ],
        );
        assert_eq!(
            restorable_session_ids(&store.state().sessions, "a"),
            [id("a", "r1"), id("a", "r2")]
        );
        assert_eq!(
            restorable_session_ids(&store.state().sessions, "b"),
            [id("b", "r-other")]
        );
        assert!(restorable_session_ids(&store.state().sessions, "ghost").is_empty());
    }
}
