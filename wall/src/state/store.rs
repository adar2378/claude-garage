//! The wall store: the only mutable slot in the TUI (port of
//! `tui/lib/state/store.dart`). It applies events (SSE, fetches, key
//! commands) to the [`WallState`] — rendering and PTY wiring subscribe via
//! `on_change` and are pure consumers.

use std::collections::{HashMap, HashSet};

use crate::api::models::{SessionInfo, UsageInfo, WorkspaceInfo};
use crate::state::salience::{build_groups, jump_target};
use crate::state::views::{
    compute_views, derive_view_name, prune_views, view_exists, view_of, View, ViewsByWorkspace,
    DEFAULT_VIEW,
};
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
    /// `d` (spec tui-views "Detach and rejoin"): move the focused session
    /// out of a multi-session view into its own solo view (focusing it), or
    /// — if it's already alone in its view — rejoin it to the default view.
    DetachFocused,
    /// `Tab` (spec tui-views "View strip and group frame"; reduced to
    /// `"\t"` by `garage_key_string` — see `garage_command_for`'s doc note):
    /// cycle the focused workspace's views.
    CycleView,
    /// `D` (shift+d, spec tui-views "Move to a group"): open the view-picker
    /// overlay for the focused session (its views list + "new group…").
    OpenViewPicker,
    ToggleHelp,
    /// `I` (shift+i, spec tui-context-meters "Install affordance"):
    /// `POST /api/statusline/install` — an effect (the store never does
    /// IO), so the store always declines it.
    InstallStatusline,
    Quit,
}

/// Maps a garage-layer key to its command; `None` for unbound keys (which
/// are consumed silently — garage typing never reaches an agent). Enter
/// arrives as `"\n"` or `"\r"` depending on the host terminal.
///
/// `Tab` is mapped here as `"\t"` ahead of the UI wave that actually
/// delivers it: `garage_key_string` (runtime.rs) currently returns `None`
/// for `KeyCode::Tab` (nothing maps a bare Tab to a string yet), so this
/// binding is inert until that function grows a
/// `KeyCode::Tab => Some("\t".to_owned())` arm — the reservation design.md
/// refers to ("Tab-in-garage was reserved for exactly this").
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
        "d" => Some(GarageCommand::DetachFocused),
        "D" => Some(GarageCommand::OpenViewPicker),
        "\t" => Some(GarageCommand::CycleView),
        "\n" | "\r" => Some(GarageCommand::Engage),
        "?" => Some(GarageCommand::ToggleHelp),
        "I" => Some(GarageCommand::InstallStatusline),
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
    /// Bumped only when `state().views` membership actually changes —
    /// `detach_focused`, or a reconciliation that pruned a dead session out
    /// of a view. `Tab`'s view-focus-only change does NOT bump this, nor
    /// does routine reconciliation that touches nothing. This is the
    /// debounced-save hook (spec tui-views "View persistence"): the runtime
    /// wave remembers the revision after each `state::persistence::save`
    /// call and, whenever this has since advanced, (re)starts its debounce
    /// timer before calling `state::persistence::save(&path,
    /// &store.state().views)`. Kept separate from `version` so ordinary
    /// churn (focus moves, SSE ticks) never restarts the debounce.
    views_revision: u64,
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

    /// See the field doc on [`WallStore::views_revision`] (the field) — the
    /// debounced `wall.json`-save hook.
    pub fn views_revision(&self) -> u64 {
        self.views_revision
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
        let (next, views_changed) = reconcile(base, sessions);
        if views_changed {
            self.views_revision += 1;
        }
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
        let (next, views_changed) = reconcile(self.state.clone(), sessions);
        if views_changed {
            self.views_revision += 1;
        }
        self.set(next);
    }

    /// `GET /api/usage` refetch (spec tui-context-meters "Strip usage
    /// chip") — account-wide data, no session reconciliation involved.
    pub fn usage_fetched(&mut self, usage: UsageInfo) {
        let mut next = self.state.clone();
        next.usage = usage;
        self.set(next);
    }

    /// Load persisted view assignments (spec tui-views "View persistence":
    /// "load-or-default... never an error" — the caller,
    /// `state::persistence::load`, already reduces a missing/invalid file
    /// to the empty map). Safe to call before or after the first
    /// `sessions_fetched`: assignments for sessions that don't exist (yet,
    /// or anymore) are pruned against whatever `state().sessions` currently
    /// holds — the exact same `reconcile` pass a later refetch runs, so a
    /// load that changes nothing never bumps `views_revision` (no
    /// redundant echo-write back to `wall.json` on startup).
    pub fn load_views(&mut self, views: ViewsByWorkspace) {
        let mut base = self.state.clone();
        base.views = views;
        let sessions = base.sessions.clone();
        let (next, views_changed) = reconcile(base, sessions);
        if views_changed {
            self.views_revision += 1;
        }
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
        let (next, views_changed) = reconcile(self.state.clone(), sessions);
        if views_changed {
            self.views_revision += 1;
        }
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

    /// Focus a session by id, switching workspace AND view and swapping the
    /// session into the (workspace, view)-scoped grid (LRU eviction) when
    /// needed. Alias of [`WallStore::focus_view_of`], kept so existing call
    /// sites (rail clicks, `p8` code) get view-awareness for free.
    pub fn focus_session(&mut self, id: &str) {
        self.focus_view_of(id);
    }

    /// Focus a session by id, switching workspace AND its owning view — the
    /// primitive behind rail clicks and both jump commands (spec tui-views
    /// "Salience is never trapped by views": "Focusing a session in a
    /// background view ... SHALL focus that session's view"). Returns false
    /// (state untouched) when the session doesn't exist.
    pub fn focus_view_of(&mut self, id: &str) -> bool {
        let Some(next) = focus_session_state(self.state.clone(), id) else {
            return false;
        };
        self.set(next);
        true
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

    // ── Views ─────────────────────────────────────────────────────────────

    /// `d` (spec tui-views "Detach and rejoin"): move the focused session
    /// out of a multi-session view into its own solo view (named after its
    /// label) and focus that view; on a session already alone in its view,
    /// rejoin it to the default view and focus the default view. Returns
    /// false (state untouched) outside the garage layer or with no focused
    /// session (empty wall).
    pub fn detach_focused(&mut self) -> bool {
        if self.state.layer != KeyLayer::Garage {
            return false;
        }
        let Some(session_id) = self.state.focused_session_id.clone() else {
            return false;
        };
        let Some(session) = self.state.session_by_id(Some(&session_id)).cloned() else {
            return false;
        };
        let workspace = session.workspace.clone();
        let mut named = self.state.views.get(&workspace).cloned().unwrap_or_default();
        let current_view = view_of(&named, &session_id);
        let current_view_size = self.state.view_session_ids(&workspace, &current_view).len();

        let target_view = if current_view_size > 1 {
            // Multi-session (default or named) view: peel this session off
            // into a brand-new solo view. Removing it can never empty
            // `current_view` here (it had >1 member), so no view needs
            // dropping on this branch.
            for v in named.iter_mut() {
                v.session_ids.retain(|id| id != &session_id);
            }
            named.retain(|v| !v.session_ids.is_empty());
            let existing_names: Vec<String> = named.iter().map(|v| v.name.clone()).collect();
            let solo_name = derive_view_name(&session.label, &existing_names);
            named.push(View { name: solo_name.clone(), session_ids: vec![session_id.clone()] });
            solo_name
        } else {
            // Alone already (in the default view or a solo view): rejoin
            // the default. A solo view left with zero members is dropped
            // (spec: "Empty non-default views SHALL be removed
            // automatically"); a no-op when it was already the default.
            named.retain(|v| v.name != current_view);
            DEFAULT_VIEW.to_owned()
        };

        self.views_revision += 1;
        let mut next = self.state.clone();
        next.views.insert(workspace.clone(), named);
        let next = focus_in_view(next, &workspace, &target_view, &session_id);
        self.set(next);
        true
    }

    /// `Tab` (spec tui-views "View strip and group frame"): cycle the
    /// focused workspace's views — default first, then named views in
    /// creation order, matching the strip's own list
    /// ([`WallState::views_for`]); the focused tile follows into the newly
    /// focused view. A no-op only when there's truly nowhere else to go: no
    /// focused workspace, the workspace has no sessions at all, or it has
    /// exactly one real (non-empty) view and focus is already on it.
    ///
    /// Never dead-ends even if `focused_view` is currently pointing at an
    /// empty view (which [`refocus_vacated_views`] should already prevent on
    /// every reconcile, but this stays a second, independent guard): that
    /// case isn't in `names` at all, so it cycles onto the first real view —
    /// including when that's the *only* one, rather than treating "fewer
    /// than two real views" as nothing-to-cycle and refusing to move.
    pub fn cycle_view(&mut self) {
        if self.state.layer != KeyLayer::Garage {
            return;
        }
        let Some(workspace) = self.state.focused_workspace.clone() else {
            return;
        };
        let names: Vec<String> =
            self.state.views_for(&workspace).into_iter().map(|v| v.name).collect();
        if names.is_empty() {
            return; // the workspace itself has no sessions — nothing to cycle to.
        }
        let current = self.state.focused_view_name(&workspace).to_owned();
        let target_view = match names.iter().position(|n| *n == current) {
            Some(pos) => names[(pos + 1) % names.len()].clone(),
            // Focused view isn't a real (non-empty) one — land on the first
            // real view instead of no-op'ing, even if it's the only one.
            None => names[0].clone(),
        };
        if target_view == current {
            return; // exactly one real view, already focused on it.
        }
        let Some(first) = self.state.view_session_ids(&workspace, &target_view).into_iter().next()
        else {
            return; // unreachable: views_for never returns an empty view.
        };
        let next = focus_in_view(self.state.clone(), &workspace, &target_view, &first);
        self.set(next);
    }

    /// The view-strip click target: focus `view_name` directly in the
    /// focused workspace, landing on its first session in salience order
    /// (same shape as [`WallStore::cycle_view`]'s destination, minus the
    /// cycling). A no-op outside the garage layer, with no focused
    /// workspace, or when the view doesn't currently exist (or is empty).
    pub fn focus_view(&mut self, view_name: &str) -> bool {
        if self.state.layer != KeyLayer::Garage {
            return false;
        }
        let Some(workspace) = self.state.focused_workspace.clone() else {
            return false;
        };
        let Some(first) = self.state.view_session_ids(&workspace, view_name).into_iter().next()
        else {
            return false;
        };
        let next = focus_in_view(self.state.clone(), &workspace, view_name, &first);
        self.set(next);
        true
    }

    /// `D` (spec tui-views "Move to a group"): open the view-picker overlay
    /// for the focused session. A no-op with no focused session (empty
    /// wall) — same guard as `d`.
    pub fn open_view_picker(&mut self) -> bool {
        if self.state.layer != KeyLayer::Garage || self.state.focused_session_id.is_none() {
            return false;
        }
        self.open_overlay(OverlayKind::ViewPicker);
        true
    }

    /// The view-picker's selection (spec tui-views "Move to a group"): move
    /// the focused session into `view_name`, creating it if it doesn't
    /// already exist, and focus it. Moving a session out of a non-default
    /// view that only had that one session removes the now-empty view (same
    /// rule as [`WallStore::detach_focused`]). `view_name ==
    /// `[views::DEFAULT_VIEW]`` simply drops the session from every named
    /// view — the default is never itself stored. A no-op outside the
    /// garage layer or with no focused session.
    pub fn move_focused_to_view(&mut self, view_name: &str) -> bool {
        if self.state.layer != KeyLayer::Garage {
            return false;
        }
        let Some(session_id) = self.state.focused_session_id.clone() else {
            return false;
        };
        let Some(session) = self.state.session_by_id(Some(&session_id)).cloned() else {
            return false;
        };
        let workspace = session.workspace.clone();
        let mut named = self.state.views.get(&workspace).cloned().unwrap_or_default();
        for v in named.iter_mut() {
            v.session_ids.retain(|id| id != &session_id);
        }
        named.retain(|v| !v.session_ids.is_empty());
        if view_name != DEFAULT_VIEW {
            match named.iter_mut().find(|v| v.name == view_name) {
                Some(v) => v.session_ids.push(session_id.clone()),
                None => named.push(View {
                    name: view_name.to_owned(),
                    session_ids: vec![session_id.clone()],
                }),
            }
        }

        self.views_revision += 1;
        let mut next = self.state.clone();
        next.views.insert(workspace.clone(), named);
        let next = focus_in_view(next, &workspace, view_name, &session_id);
        self.set(next);
        true
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
            | GarageCommand::InstallStatusline
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
            GarageCommand::DetachFocused => self.detach_focused(),
            GarageCommand::CycleView => {
                self.cycle_view();
                true
            }
            GarageCommand::OpenViewPicker => self.open_view_picker(),
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

/// Rebuild groups from `sessions` and reconcile every derived slot: views
/// pruned of dead sessions (spec tui-views "View persistence": "prune...
/// on load AND on refetch"), a dangling focused-view name reset to the
/// default, focused workspace still exists (else first group), the
/// (workspace, view)-scoped grid pruned/refilled, focused session still
/// valid, engagement dropped if its session died. Returns whether view
/// membership actually changed (pruning removed something) — the caller
/// bumps `WallStore::views_revision` on `true`, the debounced-save hook.
fn reconcile(base: WallState, sessions: Vec<WallSession>) -> (WallState, bool) {
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
fn refocus_vacated_views(mut next: WallState) -> WallState {
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
fn with_focused_workspace(base: WallState, name: Option<&str>) -> WallState {
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
fn focus_in_view(base: WallState, workspace: &str, view: &str, session_id: &str) -> WallState {
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
fn focus_session_state(base: WallState, id: &str) -> Option<WallState> {
    let session = base.session_by_id(Some(id))?;
    let workspace = session.workspace.clone();
    let named = base.views.get(&workspace).cloned().unwrap_or_default();
    let view = view_of(&named, id);
    Some(focus_in_view(base, &workspace, &view, id))
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
            title: None,
            context: None,
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
    fn usage_fetched_replaces_the_account_wide_usage_and_nothing_else() {
        use crate::api::models::UsageWindow;

        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert_eq!(store.state().usage, UsageInfo::default());
        let before_version = store.version();

        store.usage_fetched(UsageInfo {
            five_hour: Some(UsageWindow { used_percentage: 24, resets_at: None }),
            seven_day: None,
        });
        assert_eq!(
            store.state().usage.five_hour.as_ref().map(|w| w.used_percentage),
            Some(24)
        );
        assert_eq!(store.state().usage.seven_day, None);
        assert!(store.version() > before_version);
        // Sessions/focus untouched by an account-wide usage refetch.
        assert_eq!(store.state().focused_session_id, Some(id("a", "one")));
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

    // ── p11: install-statusline key (spec tui-context-meters "Install
    // affordance") ────────────────────────────────────────────────────────

    #[test]
    fn shift_i_installs_statusline_lowercase_i_stays_free() {
        assert_eq!(
            garage_command_for("I"),
            Some(GarageCommand::InstallStatusline)
        );
        assert_eq!(garage_command_for("i"), None, "lowercase i is unbound");
    }

    #[test]
    fn install_statusline_is_always_declined_by_dispatch() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert!(!store.dispatch(GarageCommand::InstallStatusline));
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

    // ── views (p10): detach/rejoin, cycle, per-view grid, persistence hook ──

    fn eight() -> Vec<SessionInfo> {
        (1..=8).map(|i| si("a", &format!("s{i}"))).collect()
    }

    #[test]
    fn p10_new_session_joins_the_default_view_leaving_other_views_unchanged() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "two"));
        assert!(store.detach_focused()); // "two" -> its own solo view "two"

        // A new session arrives (spawn/restore/unregistered discovery all
        // reconcile the same way): it must land in the default view, and
        // the solo view must be untouched.
        store.sessions_fetched(vec![si("a", "one"), si("a", "two"), si("a", "three")]);
        let views = store.state().views_for("a");
        let main = views.iter().find(|v| v.name == DEFAULT_VIEW).unwrap();
        let solo = views.iter().find(|v| v.name == "two").unwrap();
        assert_eq!(main.session_ids, [id("a", "one"), id("a", "three")]);
        assert_eq!(solo.session_ids, [id("a", "two")]);
    }

    #[test]
    fn p10_d_detaches_a_multi_session_view_to_a_solo_view_and_back() {
        // Spec tui-views "Detach and rejoin", scenario "d detaches to a
        // solo view".
        let mut store =
            store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two"), si("a", "three")]);
        store.focus_session(&id("a", "three"));

        assert!(store.detach_focused());
        assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
        assert_eq!(store.state().focused_view_name("a"), "three");
        assert_eq!(store.state().gridded_session_ids, [id("a", "three")]);
        let origin = store
            .state()
            .views_for("a")
            .into_iter()
            .find(|v| v.name == DEFAULT_VIEW)
            .unwrap();
        assert_eq!(origin.session_ids.len(), 2, "the origin view has 2 sessions");

        assert!(store.detach_focused()); // pressing d again
        assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
        assert_eq!(store.state().focused_session_id, Some(id("a", "three")));
        assert!(store.state().views.get("a").is_none_or(Vec::is_empty));
    }

    #[test]
    fn p10_detach_names_the_solo_view_after_the_label_avoiding_the_reserved_default_name() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "main"), si("a", "other")]);
        store.focus_session(&id("a", "main"));
        assert!(store.detach_focused());
        // "main" is reserved for the default view — derive_view_name parity.
        assert_eq!(store.state().focused_view_name("a"), "main-2");
    }

    #[test]
    fn p10_an_empty_non_default_view_is_removed_when_its_last_session_dies() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "two"));
        store.detach_focused();
        assert_eq!(store.state().views_for("a").len(), 2);

        store.sessions_fetched(vec![si("a", "one")]); // "two" is gone entirely
        assert_eq!(store.state().views_for("a").len(), 1);
        assert!(store.state().views.get("a").is_none_or(Vec::is_empty));
        assert_eq!(
            store.state().focused_view_name("a"),
            DEFAULT_VIEW,
            "a focused view pruned out from under it falls back to the default"
        );
        assert_eq!(store.state().focused_session_id, Some(id("a", "one")));
    }

    #[test]
    fn p10_grid_cap_and_lru_are_scoped_to_the_focused_view_not_the_whole_workspace() {
        // 8 sessions in the default view — one already over the 6-cap.
        let mut store = store_with(vec![ws("a")], eight());
        assert_eq!(store.state().gridded_session_ids.len(), 6);

        // Detach the overflow session s8 into its own solo view.
        store.focus_session(&id("a", "s8"));
        assert!(store.detach_focused());
        assert_eq!(store.state().gridded_session_ids, [id("a", "s8")], "a solo view is never capped");

        // The default view still holds 7 (s1..s7) — the cap still applies
        // to it alone, unaffected by the sibling solo view.
        store.cycle_view();
        assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
        assert_eq!(store.state().gridded_session_ids.len(), 6);
        assert!(!store.state().gridded_session_ids.contains(&id("a", "s8")));
        let rail: Vec<&str> =
            store.state().groups[0].sessions.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(rail.len(), 8, "the rail still lists every session across every view");
    }

    #[test]
    fn p10_tab_cycles_the_focused_workspaces_views_and_focus_follows() {
        let mut store =
            store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two"), si("a", "three")]);
        store.focus_session(&id("a", "three"));
        store.detach_focused();
        assert_eq!(store.state().focused_view_name("a"), "three");

        store.cycle_view();
        assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
        assert_eq!(store.state().focused_session_id, Some(id("a", "one")));

        store.cycle_view();
        assert_eq!(store.state().focused_view_name("a"), "three");
        assert_eq!(store.state().focused_session_id, Some(id("a", "three")));
    }

    #[test]
    fn p10_tab_is_a_no_op_with_a_single_view() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        let before = store.version();
        store.cycle_view();
        assert_eq!(store.version(), before, "one view: nothing to cycle to");
    }

    #[test]
    fn p10_jump_to_longest_waiting_crosses_views_focusing_the_targets_view() {
        // Spec tui-views "Salience is never trapped by views", scenario
        // "a-jump crosses views".
        let mut store = store_with(
            vec![ws("a"), ws("b")],
            vec![si("a", "one"), si("b", "keep"), si("b", "later")],
        );
        store.focus_session(&id("b", "later"));
        assert!(store.detach_focused()); // "later" -> its own solo view "later"
        store.focus_workspace_named("a");
        assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));

        store.sessions_fetched(vec![
            si("a", "one"),
            si("b", "keep"),
            si_status("b", "later", "needs-input", Some(500)),
        ]);
        assert!(store.jump_to_longest_waiting());
        assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
        assert_eq!(store.state().focused_view_name("b"), "later");
        assert_eq!(store.state().focused_session_id, Some(id("b", "later")));
        assert_eq!(store.state().layer, KeyLayer::Engaged);
    }

    #[test]
    fn p10_focus_session_crosses_views_within_the_same_workspace_rail_click_semantics() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "two"));
        store.detach_focused(); // "two" -> solo view "two", focused
        store.cycle_view(); // back to the default view, focus on "one"
        assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);

        // A rail click (or any `focus_session` call) on a session sitting
        // in a background view must bring that view into focus too.
        store.focus_view_of(&id("a", "two"));
        assert_eq!(store.state().focused_view_name("a"), "two");
        assert_eq!(store.state().focused_session_id, Some(id("a", "two")));
        assert_eq!(store.state().gridded_session_ids, [id("a", "two")]);
    }

    #[test]
    fn p10_rail_ordering_and_blocked_count_are_view_independent() {
        let mut store =
            store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two"), si("a", "three")]);
        store.focus_session(&id("a", "three"));
        store.detach_focused(); // "three" -> its own view; still the same workspace/rail.

        store.sessions_fetched(vec![
            si("a", "one"),
            si_status("a", "two", "needs-input", Some(10)),
            si_status("a", "three", "needs-input", Some(5)),
        ]);
        assert_eq!(store.state().blocked_count(), 2, "blocked count spans every view");
        let labels: Vec<&str> = store.state().groups[0].sessions.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            ["two", "three", "one"],
            "salience partitions needs-input first regardless of view membership"
        );
    }

    #[test]
    fn p10_load_views_prunes_dead_sessions_and_is_idempotent_no_spurious_dirty_bump() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        let mut loaded: HashMap<String, Vec<View>> = HashMap::new();
        loaded.insert(
            "a".to_owned(),
            vec![View {
                name: "solo".to_owned(),
                session_ids: vec![id("a", "two"), "garage/a/ghost".to_owned()],
            }],
        );
        let before = store.views_revision();
        store.load_views(loaded);
        assert_eq!(
            store.state().views.get("a").unwrap(),
            &[View { name: "solo".to_owned(), session_ids: vec![id("a", "two")] }],
            "the dead 'ghost' id is pruned on load"
        );
        assert!(store.views_revision() > before, "pruning something is a dirty change");

        let after_load = store.views_revision();
        store.sessions_fetched(vec![si("a", "one"), si("a", "two")]); // nothing new to prune
        assert_eq!(
            store.views_revision(),
            after_load,
            "a refetch that prunes nothing must not bump the debounced-save hook"
        );
    }

    #[test]
    fn p10_detach_bumps_views_revision_cycle_view_does_not() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        let before = store.views_revision();
        store.focus_session(&id("a", "two"));
        store.detach_focused();
        assert!(store.views_revision() > before);

        let after_detach = store.views_revision();
        store.cycle_view();
        assert_eq!(
            store.views_revision(),
            after_detach,
            "Tab only moves focus, never membership — never dirty"
        );
    }

    #[test]
    fn p10_d_is_a_no_op_with_an_empty_wall_or_outside_the_garage_layer() {
        let mut empty = store_with(vec![ws("a")], vec![]);
        assert!(!empty.detach_focused());

        let mut engaged = store_with(vec![ws("a")], vec![si("a", "one")]);
        engaged.engage();
        assert!(!engaged.detach_focused());
        assert_eq!(engaged.state().layer, KeyLayer::Engaged);
    }

    #[test]
    fn p10_d_and_tab_map_to_their_commands_and_dispatch() {
        assert_eq!(garage_command_for("d"), Some(GarageCommand::DetachFocused));
        assert_eq!(garage_command_for("\t"), Some(GarageCommand::CycleView));

        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        assert!(store.dispatch(GarageCommand::DetachFocused));
        assert_eq!(store.state().focused_view_name("a"), "one");
        assert!(store.dispatch(GarageCommand::CycleView)); // 2 views now: cycles.
    }

    // ── move_focused_to_view / D (spec tui-views "Move to a group") ─────

    #[test]
    fn p10_shift_d_maps_to_open_view_picker_and_opens_the_overlay() {
        assert_eq!(garage_command_for("D"), Some(GarageCommand::OpenViewPicker));
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert!(store.dispatch(GarageCommand::OpenViewPicker));
        assert_eq!(store.state().overlay, Some(OverlayKind::ViewPicker));
        assert_eq!(store.state().keys_target_chip(), "keys → move to group");
    }

    #[test]
    fn p10_shift_d_is_a_no_op_on_an_empty_wall() {
        let mut store = store_with(vec![ws("a")], vec![]);
        assert!(!store.dispatch(GarageCommand::OpenViewPicker));
        assert_eq!(store.state().overlay, None);
    }

    #[test]
    fn p10_move_focused_to_view_creates_a_new_group_and_moves_more_sessions_into_it() {
        // Spec tui-views "Move to a group", scenario "Building a
        // three-session group".
        let mut store =
            store_with(vec![ws("a")], vec![si("a", "A"), si("a", "B"), si("a", "C")]);
        store.focus_session(&id("a", "A"));
        assert!(store.move_focused_to_view("backend"));
        assert_eq!(store.state().focused_view_name("a"), "backend");
        assert_eq!(store.state().focused_session_id, Some(id("a", "A")));

        store.focus_view_of(&id("a", "B"));
        assert!(store.move_focused_to_view("backend"));
        store.focus_view_of(&id("a", "C"));
        assert!(store.move_focused_to_view("backend"));

        let views = store.state().views_for("a");
        let names: Vec<&str> = views.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["backend"], "every session left main for backend");
        let backend = views.iter().find(|v| v.name == "backend").unwrap();
        assert_eq!(
            backend.session_ids,
            [id("a", "A"), id("a", "B"), id("a", "C")]
        );
    }

    #[test]
    fn p10_moving_the_last_session_out_of_a_view_removes_it() {
        // Spec tui-views "Move to a group", scenario "Move-out removes empty
        // views".
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "two"));
        store.detach_focused(); // "two" -> its own solo view "two"
        assert_eq!(store.state().views_for("a").len(), 2);

        assert!(store.move_focused_to_view(DEFAULT_VIEW));
        assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
        let views = store.state().views_for("a");
        assert_eq!(views.len(), 1, "solo-x is gone, only main remains");
        assert!(views.iter().all(|v| v.name != "two"));
    }

    #[test]
    fn p10_move_focused_to_view_into_an_existing_named_view_merges_in() {
        let mut store = store_with(
            vec![ws("a")],
            vec![si("a", "one"), si("a", "two"), si("a", "three")],
        );
        store.focus_session(&id("a", "one"));
        store.detach_focused(); // "one" -> solo view "one"
        store.focus_view_of(&id("a", "two"));
        assert!(store.move_focused_to_view("one"));
        let view = store
            .state()
            .views_for("a")
            .into_iter()
            .find(|v| v.name == "one")
            .unwrap();
        assert_eq!(view.session_ids, [id("a", "one"), id("a", "two")]);
    }

    #[test]
    fn p10_move_focused_to_view_bumps_views_revision_and_requires_the_garage_layer() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.engage();
        assert!(!store.move_focused_to_view("backend"));

        store.disengage();
        let before = store.views_revision();
        assert!(store.move_focused_to_view("backend"));
        assert!(store.views_revision() > before);
    }

    // ── focus_view (view-strip click target) ─────────────────────────────

    #[test]
    fn p10_focus_view_lands_on_the_views_first_session_and_focuses_it() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "two"));
        store.detach_focused(); // "two" -> solo view "two"
        assert_eq!(store.state().focused_view_name("a"), "two");

        assert!(store.focus_view(DEFAULT_VIEW));
        assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
        assert_eq!(store.state().focused_session_id, Some(id("a", "one")));

        assert!(store.focus_view("two"));
        assert_eq!(store.state().focused_view_name("a"), "two");
        assert_eq!(store.state().focused_session_id, Some(id("a", "two")));
    }

    #[test]
    fn p10_focus_view_is_a_no_op_for_an_unknown_view_or_no_focused_workspace() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        assert!(!store.focus_view("ghost"));
        assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
    }

    // ── bug fix: a fully-vacated default view must never trap focus ────────
    // (verification.md "Wall bug found during 4.2": every session `D`-moved
    // out of a workspace's default view, then a fresh TUI process starts —
    // `focused_view` isn't persisted, so it assumes DEFAULT_VIEW, which now
    // has zero members and nothing (`Tab`, `]`) could ever reach the
    // populated named view). Covers (a) reconcile/load refocus, (b)
    // `cycle_view` never dead-ending, and (c) the empty-workspace hint being
    // driven by a state field that's fixed by (a), so it never fires for an
    // empty default sitting next to a populated named view.

    #[test]
    fn p10_bug_restart_shape_load_views_with_everything_in_a_named_view_focuses_it() {
        // The exact repro: two sessions, both already assigned (as if
        // restored from `wall.json`) to "backend", loaded into a brand-new
        // store whose `focused_view` map is empty (so it starts out assuming
        // DEFAULT_VIEW, per-workspace, same as a real process restart).
        let mut store = store_with(vec![ws("a")], vec![si("a", "alpha"), si("a", "beta")]);
        assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW, "sanity: starts on the default");

        let mut loaded: HashMap<String, Vec<View>> = HashMap::new();
        loaded.insert(
            "a".to_owned(),
            vec![View {
                name: "backend".to_owned(),
                session_ids: vec![id("a", "alpha"), id("a", "beta")],
            }],
        );
        store.load_views(loaded);

        assert_eq!(
            store.state().focused_view_name("a"),
            "backend",
            "the default is now empty; focus must land on the only real view"
        );
        assert_eq!(store.state().gridded_session_ids, [id("a", "alpha"), id("a", "beta")]);
        assert_eq!(store.state().focused_session_id, Some(id("a", "alpha")));
    }

    #[test]
    fn p10_bug_reconcile_refocuses_when_a_live_death_vacates_the_focused_view_and_default_is_also_empty() {
        // Same trap, reached via a live refetch instead of a cold load: two
        // named views, default empty, focus on "frontend"; its only member
        // dies. Pruning drops "frontend" (spec: empty non-default views are
        // removed), which would normally fall back to DEFAULT_VIEW — but
        // DEFAULT_VIEW is *also* still empty, so it must keep going to
        // "backend" instead of landing in the same trap by another route.
        let mut store = store_with(vec![ws("a")], vec![si("a", "x"), si("a", "y")]);
        store.focus_session(&id("a", "x"));
        assert!(store.move_focused_to_view("backend"));
        store.focus_view_of(&id("a", "y"));
        assert!(store.move_focused_to_view("frontend"));
        assert_eq!(store.state().focused_view_name("a"), "frontend");

        store.sessions_fetched(vec![si("a", "x")]); // "y" (frontend's only member) dies
        assert_eq!(
            store.state().focused_view_name("a"),
            "backend",
            "default is still empty too — must land on the only real view left"
        );
        assert_eq!(store.state().gridded_session_ids, [id("a", "x")]);
    }

    #[test]
    fn p10_bug_refocus_is_a_no_op_when_the_focused_view_already_has_members() {
        // Guard against over-firing: a normal, already-correct focus must
        // never be disturbed by the refocus pass.
        let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
        store.focus_session(&id("a", "two"));
        assert!(store.detach_focused()); // "two" -> solo view "two", non-empty default remains
        assert_eq!(store.state().focused_view_name("a"), "two");

        store.sessions_fetched(vec![si("a", "one"), si("a", "two")]); // no-op refetch
        assert_eq!(store.state().focused_view_name("a"), "two", "already on a real view — untouched");
    }

    #[test]
    fn p10_bug_a_truly_empty_workspace_is_left_on_the_default_not_redirected() {
        // No sessions at all: nothing non-empty to redirect to, so the
        // (distinct, correct) "no workspaces/sessions" render path is
        // untouched by the refocus pass.
        let store = store_with(vec![ws("a")], vec![]);
        assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
        assert!(store.state().gridded_session_ids.is_empty());
    }

    #[test]
    fn p10_bug_cycle_view_escapes_an_empty_focused_view_even_with_only_one_real_view_left() {
        // Direct unit test of cycle_view's own dead-end guard (b), forcing
        // exactly the stuck shape independent of the reconcile-side fix (a):
        // `focused_view` pointing at the now-empty default while "backend"
        // is the workspace's only real view.
        let mut store = store_with(vec![ws("a")], vec![si("a", "alpha"), si("a", "beta")]);
        store.focus_session(&id("a", "alpha"));
        assert!(store.move_focused_to_view("backend"));
        store.focus_view_of(&id("a", "beta"));
        assert!(store.move_focused_to_view("backend"));
        store.state.focused_view.insert("a".to_owned(), DEFAULT_VIEW.to_owned());
        assert_eq!(store.state().views_for("a").len(), 1, "sanity: only \"backend\" is real");

        store.cycle_view();

        assert_eq!(store.state().focused_view_name("a"), "backend");
        assert_eq!(store.state().gridded_session_ids, [id("a", "alpha"), id("a", "beta")]);
    }

    #[test]
    fn p10_bug_cycle_view_stays_put_with_a_single_real_view_already_focused() {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        store.focus_session(&id("a", "one"));
        assert!(store.move_focused_to_view("backend")); // the only view, already focused
        let before = store.version();

        store.cycle_view();

        assert_eq!(store.version(), before, "one real view, already on it: nothing to cycle to");
    }

    #[test]
    fn p10_bug_vacated_default_view_never_renders_the_empty_workspace_hint() {
        // runtime.rs's "no sessions in this workspace" hint fires purely off
        // `gridded_session_ids.is_empty()` once a workspace group exists at
        // all — it has no idea about `views`/`focused_view`. So the (a) fix
        // must keep that field non-empty here, or an empty *default* view
        // would wrongly render as though the whole *workspace* were empty.
        let mut store = store_with(vec![ws("a")], vec![si("a", "alpha"), si("a", "beta")]);
        let mut loaded: HashMap<String, Vec<View>> = HashMap::new();
        loaded.insert(
            "a".to_owned(),
            vec![View {
                name: "backend".to_owned(),
                session_ids: vec![id("a", "alpha"), id("a", "beta")],
            }],
        );
        store.load_views(loaded);

        assert!(
            !store.state().gridded_session_ids.is_empty(),
            "must never look empty — the hint would wrongly claim the workspace has no sessions"
        );

        // Contrast: a workspace with genuinely zero sessions legitimately
        // renders an empty grid — the hint is correct there, untouched.
        let empty_store = store_with(vec![ws("b")], vec![]);
        assert!(empty_store.state().gridded_session_ids.is_empty());
    }
}
