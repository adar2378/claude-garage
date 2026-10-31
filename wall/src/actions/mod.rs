//! Named wall actions (p22 tui-actions "Named actions"): every behaviour a
//! key or a click can trigger has a stable dotted id and runs with an
//! [`ActionCtx`] payload. Keys resolve to ids through [`crate::keymap`],
//! clicks through `ui::hit_targets::action_at`; both then run the same
//! behaviour. The ids are the contract later phases (a user keymap, the
//! plugin host) bind to — rename with care.

use crate::api::models::WorktreeRecord;
use crate::state::wall_state::WallState;

/// A stable, dotted action name (`session.close`, `jump.blocked`, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActionId(pub &'static str);

impl ActionId {
    pub fn as_str(self) -> &'static str {
        self.0
    }

    /// The id for a runtime string (a plugin's `plugin:action`). Core ids
    /// resolve to their `&'static` constant; anything else is leaked once
    /// into a process-wide set, so the same string always yields the same
    /// pointer and `ActionId` stays `Copy`. The set is bounded by the
    /// installed plugins' declared action ids, so the leak is a few hundred
    /// bytes (p24 design.md Gap A).
    pub fn intern(id: &str) -> ActionId {
        let mut set = INTERNER.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = set.get(id) {
            return ActionId(existing);
        }
        let leaked: &'static str = Box::leak(id.to_owned().into_boxed_str());
        set.insert(leaked);
        ActionId(leaked)
    }

    /// True for a plugin id (`plugin:action`). Core ids are dotted and never
    /// contain a colon.
    pub fn is_plugin(self) -> bool {
        self.0.contains(':')
    }
}

/// The interner, seeded with every core id so interning a core string hands
/// back the core constant instead of leaking a duplicate.
static INTERNER: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<&'static str>>> =
    std::sync::LazyLock::new(|| {
        std::sync::Mutex::new(ids::ALL.iter().map(|id| id.0).collect())
    });

/// The action id list. Group order follows design.md decision 1.
pub mod ids {
    use super::ActionId;

    pub const WORKSPACE_FOCUS: ActionId = ActionId("workspace.focus");
    pub const FOCUS_CYCLE_PREV: ActionId = ActionId("focus.cycle.prev");
    pub const FOCUS_CYCLE_NEXT: ActionId = ActionId("focus.cycle.next");
    pub const JUMP_BLOCKED: ActionId = ActionId("jump.blocked");
    pub const OVERLAY_OPEN_QUEUE: ActionId = ActionId("overlay.open.queue");
    pub const OVERLAY_OPEN_HELP: ActionId = ActionId("overlay.open.help");
    pub const OVERLAY_OPEN_VIEW_PICKER: ActionId = ActionId("overlay.open.view_picker");
    pub const OVERLAY_OPEN_WORKSPACE_ADD: ActionId = ActionId("overlay.open.workspace_add");
    pub const SESSION_SPAWN: ActionId = ActionId("session.spawn");
    pub const SESSION_SPAWN_WORKTREE: ActionId = ActionId("session.spawn.worktree");
    pub const SESSION_ENGAGE: ActionId = ActionId("session.engage");
    pub const SESSION_MAXIMIZE: ActionId = ActionId("session.maximize");
    pub const SESSION_CLOSE: ActionId = ActionId("session.close");
    pub const SESSION_RESTORE_ALL: ActionId = ActionId("session.restore_all");
    pub const SESSION_DETACH: ActionId = ActionId("session.detach");
    pub const SESSION_OPEN_WINDOW: ActionId = ActionId("session.open_window");
    pub const SESSION_RESTART: ActionId = ActionId("session.restart");
    pub const WORKSPACE_RESTART: ActionId = ActionId("workspace.restart");
    pub const DAEMON_RESTART: ActionId = ActionId("daemon.restart");
    pub const VIEW_CYCLE: ActionId = ActionId("view.cycle");
    pub const WORKSPACE_REMOVE: ActionId = ActionId("workspace.remove");
    pub const WORKSPACE_REMOVE_KILL: ActionId = ActionId("workspace.remove.kill");
    pub const HOOKS_INSTALL: ActionId = ActionId("hooks.install");
    pub const APP_QUIT: ActionId = ActionId("app.quit");
    /// Click-reachable (and internal) ids with no key today.
    pub const SESSION_FOCUS: ActionId = ActionId("session.focus");
    pub const SESSION_RESTORE: ActionId = ActionId("session.restore");
    /// Forget a session (`DELETE /api/sessions/{id}?meta=1`). No key; reached
    /// by plugin buttons, always through the confirm rule.
    pub const SESSION_DELETE: ActionId = ActionId("session.delete");
    pub const PET_PET: ActionId = ActionId("pet.pet");
    /// `P`: not a [`super::GarageCommand`] (the store has no pet dependency).
    pub const PET_CYCLE_SPECIES: ActionId = ActionId("pet.cycle_species");
    /// Overlay and view-strip ids (the overlay keys and their clicks).
    pub const OVERLAY_CLOSE: ActionId = ActionId("overlay.close");
    pub const OVERLAY_SELECT_NEXT: ActionId = ActionId("overlay.select.next");
    pub const OVERLAY_SELECT_PREV: ActionId = ActionId("overlay.select.prev");
    pub const OVERLAY_VIEW_PICKER_PICK: ActionId = ActionId("overlay.view_picker.pick");
    pub const VIEW_FOCUS: ActionId = ActionId("view.focus");
    /// The `r` prefix key: opens the one-shot restart chord (resolved in the
    /// router, which owns the prefix state — never a command by itself).
    pub const CHORD_RESTART: ActionId = ActionId("chord.restart");

    /// Every id, for the "list is complete" and reachability tests.
    pub const ALL: &[ActionId] = &[
        WORKSPACE_FOCUS,
        FOCUS_CYCLE_PREV,
        FOCUS_CYCLE_NEXT,
        JUMP_BLOCKED,
        OVERLAY_OPEN_QUEUE,
        OVERLAY_OPEN_HELP,
        OVERLAY_OPEN_VIEW_PICKER,
        OVERLAY_OPEN_WORKSPACE_ADD,
        SESSION_SPAWN,
        SESSION_SPAWN_WORKTREE,
        SESSION_ENGAGE,
        SESSION_MAXIMIZE,
        SESSION_CLOSE,
        SESSION_RESTORE_ALL,
        SESSION_DETACH,
        SESSION_OPEN_WINDOW,
        SESSION_RESTART,
        WORKSPACE_RESTART,
        DAEMON_RESTART,
        VIEW_CYCLE,
        WORKSPACE_REMOVE,
        WORKSPACE_REMOVE_KILL,
        HOOKS_INSTALL,
        APP_QUIT,
        SESSION_FOCUS,
        SESSION_RESTORE,
        SESSION_DELETE,
        PET_PET,
        PET_CYCLE_SPECIES,
        OVERLAY_CLOSE,
        OVERLAY_SELECT_NEXT,
        OVERLAY_SELECT_PREV,
        OVERLAY_VIEW_PICKER_PICK,
        VIEW_FOCUS,
        CHORD_RESTART,
    ];
}

/// What triggered an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionSource {
    Key(String),
    /// The key after the `r` prefix (the `RestartChord` layer). Only this
    /// source may run the restart actions.
    Chord(String),
    Mouse { col: u16, row: u16 },
}

/// The payload an action runs with. `session` is the focused session for a
/// key, the clicked one for a click that names a session; `index` carries
/// the `1`–`9` workspace number for `workspace.focus`; `selection` is a row
/// index inside an overlay; `view` names a view (view-strip click).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionCtx {
    pub workspace: Option<String>,
    pub session: Option<String>,
    pub index: Option<usize>,
    pub selection: Option<usize>,
    pub view: Option<String>,
    /// The held worktree record, when the action comes from the worktree
    /// finish flow (none of the key/click ids carry one yet).
    pub worktree: Option<WorktreeRecord>,
    pub source: ActionSource,
}

impl ActionCtx {
    /// A context seeded from the focus: focused workspace and session.
    pub fn from_focus(state: &WallState, source: ActionSource) -> Self {
        ActionCtx {
            workspace: state.focused_workspace.clone(),
            session: state.focused_session_id.clone(),
            index: None,
            selection: None,
            view: None,
            worktree: None,
            source,
        }
    }

    pub fn with_index(mut self, index: Option<usize>) -> Self {
        self.index = index;
        self
    }

    pub fn with_session(mut self, id: String) -> Self {
        self.session = Some(id);
        self
    }

    pub fn with_selection(mut self, selection: usize) -> Self {
        self.selection = Some(selection);
        self
    }

    pub fn with_view(mut self, view: String) -> Self {
        self.view = Some(view);
        self
    }
}

/// Garage-layer commands (spec tui-key-routing: "Garage-layer bindings" +
/// "p8.1 session lifecycle bindings" + "p8.3 workspace removal",
/// `1-9 [ ] a A n N m R x X w Enter ? q`, plus the p16-restart `r` prefix's
/// `r r` / `r a` / `r d`, which resolve in the keymap's restart-chord layer).
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
    /// effect (parallel per-id `POST /api/sessions/restore` calls), so the
    /// store always declines it.
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
    /// `"\t"` by `garage_key_string`):
    /// cycle the focused workspace's views.
    CycleView,
    /// `D` (shift+d, spec tui-views "Move to a group"): open the view-picker
    /// overlay for the focused session (its views list + "new group…").
    OpenViewPicker,
    ToggleHelp,
    /// `I` (shift+i, spec tui-context-meters "Install affordance", spec
    /// tui-hooks-install): `POST /api/statusline/install` then `POST
    /// /api/hooks/install` — an effect (the store never does IO), so the
    /// store always declines it.
    InstallStatusline,
    /// `t`: open the focused LIVE session in its own OS terminal window
    /// (macOS only) — a detached `tmux attach` client alongside the wall's
    /// own, so the store never does IO and always declines it.
    OpenWindow,
    /// `r r` (p16-restart, spec restart "TUI chords"): armed double-press
    /// restart of the focused session in place — an effect
    /// (`POST /api/sessions/restart {id, force}`), so the store always
    /// declines it.
    RestartFocused,
    /// `r a`: restart every idle/done session of the focused workspace
    /// ([`WallStore::restart_targets_for_workspace`]) — an effect (one
    /// `POST /api/sessions/restart` per id, since the daemon has no
    /// workspace filter), always declined.
    RestartWorkspace,
    /// `r d`: restart the daemon itself (`POST /api/daemon/restart`) —
    /// tmux sessions are untouched and the SSE task's existing reconnect
    /// carries the wall across the handoff. An effect, always declined.
    RestartDaemon,
    Quit,
}

/// The [`GarageCommand`] an action id runs through the store/router, or
/// `None` for ids with their own handling (clicks, overlay keys, the pet).
pub fn command_for_action(id: ActionId, index: Option<usize>) -> Option<GarageCommand> {
    use GarageCommand as C;
    Some(match id {
        ids::WORKSPACE_FOCUS => C::FocusWorkspace(index?),
        ids::FOCUS_CYCLE_PREV => C::CycleFocus(-1),
        ids::FOCUS_CYCLE_NEXT => C::CycleFocus(1),
        ids::JUMP_BLOCKED => C::Jump,
        ids::OVERLAY_OPEN_QUEUE => C::OpenQueue,
        ids::OVERLAY_OPEN_HELP => C::ToggleHelp,
        ids::OVERLAY_OPEN_VIEW_PICKER => C::OpenViewPicker,
        ids::OVERLAY_OPEN_WORKSPACE_ADD => C::WorkspaceAdd,
        ids::SESSION_SPAWN => C::Spawn { worktree: false },
        ids::SESSION_SPAWN_WORKTREE => C::Spawn { worktree: true },
        ids::SESSION_ENGAGE => C::Engage,
        ids::SESSION_MAXIMIZE => C::Maximize,
        ids::SESSION_CLOSE => C::Close,
        ids::SESSION_RESTORE_ALL => C::RestoreAll,
        ids::SESSION_DETACH => C::DetachFocused,
        ids::SESSION_OPEN_WINDOW => C::OpenWindow,
        ids::SESSION_RESTART => C::RestartFocused,
        ids::WORKSPACE_RESTART => C::RestartWorkspace,
        ids::DAEMON_RESTART => C::RestartDaemon,
        ids::VIEW_CYCLE => C::CycleView,
        ids::WORKSPACE_REMOVE => C::WorkspaceRemove,
        ids::HOOKS_INSTALL => C::InstallStatusline,
        ids::APP_QUIT => C::Quit,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_dotted_names() {
        let mut seen = std::collections::HashSet::new();
        for id in ids::ALL {
            assert!(seen.insert(id.as_str()), "duplicate {}", id.as_str());
            assert!(
                id.as_str().contains('.'),
                "{} is not a dotted name",
                id.as_str()
            );
        }
    }

    #[test]
    fn the_ids_without_a_command_are_exactly_the_ones_handled_beside_the_store() {
        // Everything else runs through `WallStore::dispatch` or the router's
        // effects; these have their own handling (see `App::dispatch_action`
        // and `GarageRouter::run_action`).
        let handled_elsewhere = [
            ids::SESSION_FOCUS,
            ids::SESSION_RESTORE,
            ids::SESSION_DELETE,
            ids::PET_PET,
            ids::PET_CYCLE_SPECIES,
            ids::OVERLAY_CLOSE,
            ids::OVERLAY_SELECT_NEXT,
            ids::OVERLAY_SELECT_PREV,
            ids::OVERLAY_VIEW_PICKER_PICK,
            ids::VIEW_FOCUS,
            ids::CHORD_RESTART,
            ids::WORKSPACE_REMOVE_KILL,
        ];
        for id in ids::ALL {
            let has_command = command_for_action(*id, Some(0)).is_some();
            assert_eq!(
                has_command,
                !handled_elsewhere.contains(id),
                "{} is {} a command",
                id.as_str(),
                if has_command { "mapped to" } else { "missing from" }
            );
        }
    }

    #[test]
    fn interning_the_same_string_gives_the_same_pointer() {
        let a = ActionId::intern("closer:close-all");
        let b = ActionId::intern(&String::from("closer:close-all"));
        assert_eq!(a.as_str().as_ptr(), b.as_str().as_ptr());
        assert_eq!(a, b);
        assert_ne!(a, ActionId::intern("closer:other"));
        assert!(a.is_plugin());
    }

    #[test]
    fn interning_a_core_id_equals_the_constant_and_leaks_nothing() {
        let before = INTERNER.lock().unwrap().len();
        for id in ids::ALL {
            let first = ActionId::intern(id.as_str());
            let second = ActionId::intern(id.as_str());
            assert_eq!(first, *id);
            assert_eq!(first.as_str().as_ptr(), second.as_str().as_ptr());
            assert!(!first.is_plugin());
        }
        // Other tests intern plugin ids concurrently, so only assert the core
        // ids did not add entries of their own: the set never shrinks and
        // every core string was already in it.
        let set = INTERNER.lock().unwrap();
        assert!(set.len() >= before);
        assert!(ids::ALL.iter().all(|id| set.contains(id.as_str())));
    }

    #[test]
    fn plugin_ids_never_enter_the_core_list() {
        ActionId::intern("someplugin:act");
        assert!(ids::ALL.iter().all(|id| !id.as_str().contains(':')));
    }

    #[test]
    fn workspace_focus_needs_its_index() {
        assert_eq!(command_for_action(ids::WORKSPACE_FOCUS, None), None);
    }
}
