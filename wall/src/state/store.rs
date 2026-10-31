//! The wall store: the only mutable slot in the TUI (port of
//! `tui/lib/state/store.dart`). It applies events (SSE, fetches, key
//! commands) to the [`WallState`] — rendering and PTY wiring subscribe via
//! `on_change` and are pure consumers.

use std::collections::HashMap;

pub use crate::actions::GarageCommand;
use crate::api::models::{SessionInfo, UsageInfo, WorkspaceInfo};
use crate::state::armed_action::ArmedAction;
use crate::state::salience::jump_target;
use crate::state::views::{
    derive_view_name, view_of, View, ViewsByWorkspace,
    DEFAULT_VIEW,
};
use crate::state::wall_state::{KeyLayer, OverlayKind, WallSession, WallState};

/// The `r r` arming (p16-restart, spec restart "TUI chords"): the generic
/// [`ArmedAction`] machine (3s window, same as the `x x` close) plus the one
/// bit the confirming press needs — whether the target was busy when it
/// armed. Busy at ARM time, not at fire time: the arming notice warned about
/// the state the user saw, and the second press is that warning's answer, so
/// it carries `force` even if a status tick landed in between.
#[derive(Default)]
pub struct ArmedRestart {
    action: ArmedAction,
    /// Whether the CURRENT arm was made on a busy session.
    force: bool,
}

impl ArmedRestart {
    /// The session id currently armed, or `None` (rendering/tests only —
    /// [`ArmedRestart::press`] is what checks the clock).
    pub fn armed_id(&self) -> Option<&str> {
        self.action.armed_id()
    }

    /// One `r r` press aimed at `id`; `busy` = the session is
    /// `working`/`needs-input` right now. `Some(force)` means FIRE the
    /// restart (`force` = it was busy when it armed); `None` means it armed
    /// — show [`restart_arm_notice`].
    pub fn press(&mut self, id: &str, busy: bool, now_ms: i64) -> Option<bool> {
        if self.action.press(id, now_ms) {
            let force = self.force;
            self.force = false;
            return Some(force);
        }
        self.force = busy;
        None
    }

    /// Any key but the prefix disarms — the same "any other key disarms"
    /// rule the `x x` close follows.
    pub fn disarm(&mut self) {
        self.action.disarm();
        self.force = false;
    }
}

/// The `r r` arming notice (p16-restart, spec restart "TUI chords": it says
/// the conversation resumes, and warns when the session is busy — the
/// second press is then the force). `needs-input` shares the busy wording:
/// both mean a live turn would be lost.
pub fn restart_arm_notice(name: &str, busy: bool) -> String {
    if busy {
        format!("{name} is working — r r again to restart anyway")
    } else {
        format!("restart {name}? r r again — resumes the conversation")
    }
}

use crate::state::reconcile::*;

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

    /// `P` cycles the pit-pet roster (spec tui-pit-pet "Opt-in roster cycled
    /// by `P`"). `pet` is `"cat"` | `"duck"` | `"pup"` | `None` (off) — the
    /// wiring phase is the one place that knows about `Species`; the store
    /// just carries whatever it's given. Bumps `views_revision`, the same
    /// debounced-save hook views mutations use: `wall.json` holds both, so a
    /// pet change is exactly as save-worthy as a view change (see
    /// `state::persistence::WallFile`).
    pub fn set_pet(&mut self, pet: Option<String>) {
        let mut next = self.state.clone();
        next.pet = pet;
        self.views_revision += 1;
        self.set(next);
    }

    /// `AppEvent::Connection(bool)` from the SSE task (spec tui-pit-pet "One-
    /// row sprites and derived mood": "daemon SSE disconnected → box").
    /// Transient — never persisted, so this never touches `views_revision`.
    pub fn connection(&mut self, live: bool) {
        let mut next = self.state.clone();
        next.daemon_live = live;
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

    /// The `r a` target set (p16-restart, spec restart "TUI chords"): every
    /// LIVE `idle`/`done` session of the FOCUSED workspace, listing order
    /// preserved. `working` and `needs-input` are excluded — a restart
    /// mid-turn loses the turn (design.md D2) — and so are restorable
    /// placeholders, which have no tmux pane to respawn (a `restorable`
    /// status is neither `idle` nor `done`, so the same test covers them).
    /// The caller issues one `POST /api/sessions/restart` per id, the same
    /// shape as the `R` restore-all, since the daemon has no workspace
    /// filter.
    pub fn restart_targets_for_workspace(&self) -> Vec<String> {
        let Some(workspace) = self.state.focused_workspace.as_deref() else {
            return Vec::new();
        };
        self.state
            .sessions
            .iter()
            .filter(|s| s.workspace == workspace)
            .filter(|s| s.status == "idle" || s.status == "done")
            .map(|s| s.id.clone())
            .collect()
    }

    // ── Command dispatch ──────────────────────────────────────────────────

    /// Apply a garage-layer command to the state. Returns true when the
    /// store handled it; `Spawn`, `RestoreAll`, `Close`, `WorkspaceRemove`,
    /// the three p16-restart commands and `Quit` always return false — they
    /// are the caller's effects.
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
            | GarageCommand::OpenWindow
            | GarageCommand::RestartFocused
            | GarageCommand::RestartWorkspace
            | GarageCommand::RestartDaemon
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

#[cfg(test)]
mod tests;
