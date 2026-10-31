//! Wave-3 key-routing tests (spec tui-key-routing incl. p8.1–p8.4):
//! the garage router's armed `x`/`X`/`K` flows with the exact Dart
//! strip-notice wording (the e2e harnesses grep these strings verbatim),
//! the lifecycle effects, the engaged-layer encode path with the Ctrl+G
//! disengage, and the overlay placeholder routing. The store transitions
//! themselves are covered by `state/store.rs`; these tests cover the
//! caller-side behavior the Dart TUI kept in `bin/garage_tui.dart`.
use super::*;
use super::router::next_label;
use crate::keymap::garage_command_for;
use crate::api::models::{SessionInfo, WorkspaceInfo};

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

fn si_blocked(workspace: &str, label: &str, since: i64) -> SessionInfo {
    SessionInfo {
        status: "needs-input".to_owned(),
        since: Some(since),
        ..si(workspace, label)
    }
}

fn si_status(workspace: &str, label: &str, status: &str) -> SessionInfo {
    SessionInfo {
        status: status.to_owned(),
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

fn key_at(
    router: &mut GarageRouter,
    store: &mut WallStore,
    key: &str,
    now: i64,
) -> Vec<Effect> {
    let outcome = router.on_garage_key(store, key, now);
    let mut effects = outcome.effects;
    if let Some((id, ctx)) = outcome.action {
        router.run_action(store, id, &ctx, now, &mut effects);
    }
    effects
}

fn plain(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

mod dispatch;
mod layers;
mod router;
mod worktree;
