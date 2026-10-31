//! Named wall actions (p22 tui-actions "Named actions"): every behaviour a
//! key or a click can trigger has a stable dotted id and runs with an
//! [`ActionCtx`] payload. Keys resolve to ids through [`crate::keymap`],
//! clicks through `ui::hit_targets::action_at`; both then run the same
//! behaviour. The ids are the contract later phases (a user keymap, the
//! plugin host) bind to — rename with care.

use crate::api::models::WorktreeRecord;
use crate::state::store::GarageCommand;
use crate::state::wall_state::WallState;

/// A stable, dotted action name (`session.close`, `jump.blocked`, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActionId(pub &'static str);

impl ActionId {
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

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

/// The action id (and `workspace.focus` index) a [`GarageCommand`] maps to.
pub fn action_for_command(command: GarageCommand) -> (ActionId, Option<usize>) {
    use GarageCommand as C;
    let id = match command {
        C::FocusWorkspace(n) => return (ids::WORKSPACE_FOCUS, Some(n)),
        C::CycleFocus(delta) if delta < 0 => ids::FOCUS_CYCLE_PREV,
        C::CycleFocus(_) => ids::FOCUS_CYCLE_NEXT,
        C::Jump => ids::JUMP_BLOCKED,
        C::OpenQueue => ids::OVERLAY_OPEN_QUEUE,
        C::Spawn { worktree: false } => ids::SESSION_SPAWN,
        C::Spawn { worktree: true } => ids::SESSION_SPAWN_WORKTREE,
        C::Engage => ids::SESSION_ENGAGE,
        C::Maximize => ids::SESSION_MAXIMIZE,
        C::RestoreAll => ids::SESSION_RESTORE_ALL,
        C::Close => ids::SESSION_CLOSE,
        C::WorkspaceAdd => ids::OVERLAY_OPEN_WORKSPACE_ADD,
        C::WorkspaceRemove => ids::WORKSPACE_REMOVE,
        C::DetachFocused => ids::SESSION_DETACH,
        C::CycleView => ids::VIEW_CYCLE,
        C::OpenViewPicker => ids::OVERLAY_OPEN_VIEW_PICKER,
        C::ToggleHelp => ids::OVERLAY_OPEN_HELP,
        C::InstallStatusline => ids::HOOKS_INSTALL,
        C::OpenWindow => ids::SESSION_OPEN_WINDOW,
        C::RestartFocused => ids::SESSION_RESTART,
        C::RestartWorkspace => ids::WORKSPACE_RESTART,
        C::RestartDaemon => ids::DAEMON_RESTART,
        C::Quit => ids::APP_QUIT,
    };
    (id, None)
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
    fn every_garage_command_round_trips_through_its_id() {
        use GarageCommand as C;
        let commands = [
            C::FocusWorkspace(0),
            C::FocusWorkspace(8),
            C::CycleFocus(-1),
            C::CycleFocus(1),
            C::Jump,
            C::OpenQueue,
            C::Spawn { worktree: false },
            C::Spawn { worktree: true },
            C::Engage,
            C::Maximize,
            C::RestoreAll,
            C::Close,
            C::WorkspaceAdd,
            C::WorkspaceRemove,
            C::DetachFocused,
            C::CycleView,
            C::OpenViewPicker,
            C::ToggleHelp,
            C::InstallStatusline,
            C::OpenWindow,
            C::RestartFocused,
            C::RestartWorkspace,
            C::RestartDaemon,
            C::Quit,
        ];
        for command in commands {
            let (id, index) = action_for_command(command);
            assert_eq!(command_for_action(id, index), Some(command), "{id:?}");
        }
    }

    #[test]
    fn workspace_focus_needs_its_index() {
        assert_eq!(command_for_action(ids::WORKSPACE_FOCUS, None), None);
    }
}
