//! Wave-3 key-routing tests (spec tui-key-routing incl. p8.1–p8.4):
//! the garage router's armed `x`/`X`/`K` flows with the exact Dart
//! strip-notice wording (the e2e harnesses grep these strings verbatim),
//! the lifecycle effects, the engaged-layer encode path with the Ctrl+G
//! disengage, and the overlay placeholder routing. The store transitions
//! themselves are covered by `state/store.rs`; these tests cover the
//! caller-side behavior the Dart TUI kept in `bin/garage_tui.dart`.
use super::router::next_label;
use super::*;
use crate::actions::{ActionCtx, ActionSource, command_for_action, ids};
use crate::api::models::{RestartResponse, SessionInfo, WorkspaceInfo};
use crate::keymap::Layer;
use crate::state::store::GarageCommand;

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

/// A garage key through the production path: `GarageRouter::on_garage_key`,
/// then `App::dispatch_action_at` (the overlay/pet layer and the router's
/// `run_action`). The caller's router and store are swapped into a scratch
/// `App` for the call so the tests keep their virtual clock.
fn key_at(router: &mut GarageRouter, store: &mut WallStore, key: &str, now: i64) -> Vec<Effect> {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0", 0);
    std::mem::swap(&mut app.router, router);
    std::mem::swap(&mut app.store, store);
    let outcome = app
        .router
        .on_garage_key(&mut app.store, &app.keymap, key, now);
    let mut effects = outcome.effects;
    if let Some((id, ctx)) = outcome.action {
        effects.extend(app.dispatch_action_at(id, &ctx, now));
    }
    std::mem::swap(&mut app.router, router);
    std::mem::swap(&mut app.store, store);
    effects
}

fn plain(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

mod dispatch;
mod draw;
mod layers;
mod motion;
mod plugin_gate;
mod plugins;
mod router;
mod worktree;
