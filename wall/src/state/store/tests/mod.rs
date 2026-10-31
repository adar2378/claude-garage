//! Port of `tui/test/wall_store_test.dart`: layer transitions, the 6-cap
//! grid with LRU swap-in, the a-jump selector, garage command mapping,
//! and SSE-event application. Specs: tui-key-routing (layers, garage
//! bindings), tui-wall (grid cap), tui-triage (a jump).
use super::*;
use crate::actions::command_for_action;
use crate::keymap::{Keymap, Layer};
use crate::state::salience::restorable_session_ids;

/// The command a key runs in the default keymap (key → action id →
/// command); `None` for an unbound key or an id with no command.
fn command_in(layer: Layer, key: &str) -> Option<GarageCommand> {
    let resolved = Keymap::defaults().resolve(layer, key)?;
    command_for_action(resolved.action, resolved.index)
}

fn garage_command_for(key: &str) -> Option<GarageCommand> {
    command_in(Layer::Garage, key)
}

fn restart_command_for(key: &str) -> Option<GarageCommand> {
    command_in(Layer::RestartChord, key)
}

fn ws(name: &str) -> WorkspaceInfo {
    WorkspaceInfo {
        name: name.to_owned(),
        dir: Some(format!("/repos/{name}/{name}")),
        branch: None,
    }
}

fn si(workspace: &str, label: &str) -> SessionInfo {
    SessionInfo {
        id: format!("garage/{workspace}/{label}"),
        workspace: workspace.to_owned(),
        label: label.to_owned(),
        dir: Some(format!("/repos/{workspace}/{workspace}")),
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

mod misc;
mod store;
mod views;
