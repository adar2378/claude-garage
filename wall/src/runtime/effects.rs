//! Effects: the IO a store-declined command asks for, and its execution on a
//! blocking task (moved out of `runtime/mod.rs`, p22 task 1.1).

use crate::api::client::{ApiError, GarageClient};
use crate::api::models::{FinishAction, HooksInstallResult, RestartResponse, WorktreeRecord};
use crate::state::workspace_remove::kill_remove_notice;
use crate::ui::workspace_add::PickOutcome;

use super::{AppEvent, EventSender, fetch_and_send};

/// The install action's success wording (spec tui-context-meters "Install
/// affordance": exact strip-notice text, verbatim — harnesses grep it).
pub(super) const INSTALL_STATUSLINE_SUCCESS: &str =
    "statusline feed installed — meters go live as agents work";

/// Daemon API calls the garage router asks the loop to perform — the store
/// never does IO, and neither does the router: effects run on blocking tasks
/// and report back through the event channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// `n`/`N`: `POST /api/sessions`.
    Spawn {
        workspace: String,
        label: String,
        worktree: bool,
    },
    /// Enter on a restorable tile / one `R` target:
    /// `POST /api/sessions/restore {id}`.
    Restore { id: String },
    /// Confirmed `x`-`x`: `DELETE /api/sessions/<id>` (`?meta=1` for a
    /// restorable placeholder).
    Close {
        id: String,
        label: String,
        meta_only: bool,
    },
    /// A confirmed `session.delete` on a LIVE session:
    /// `DELETE /api/sessions/<id>?forget=1` kills it and keeps no history.
    Forget { id: String, label: String },
    /// Confirmed `X`-`X` (registry-only) or `X` then `K` (`kill: true`,
    /// `?sessions=kill`).
    RemoveWorkspace { name: String, kill: bool },
    /// The `w` overlay's validated submit: `PUT /api/workspaces {name, dir}`.
    AddWorkspace { name: String, dir: String },
    /// The `w` overlay's Ctrl+O: `POST /api/pick-directory` (native macOS
    /// folder dialog). Runs on its own blocking task like every effect, so
    /// the up-to-120 s dialog never stalls other effects.
    PickDirectory,
    /// `q`: orderly quit.
    Quit,
    /// `I`: `POST /api/statusline/install` then `POST /api/hooks/install`
    /// on one blocking task (spec tui-context-meters "Install affordance",
    /// spec tui-hooks-install).
    Install,
    /// The finish overlay's `m` / confirmed `d d`: `POST
    /// /api/worktrees/finish` with the held record (spec
    /// tui-worktree-finish). Keep never becomes an effect.
    FinishWorktree {
        record: WorktreeRecord,
        action: FinishAction,
    },
    /// `t`: launch a detached OS terminal window attached to the focused
    /// LIVE session (macOS only — the router declines before this is ever
    /// built off macOS; see [`crate::ui::window_open`]).
    OpenWindow { id: String, label: String },
    /// Confirmed `r r` (p16-restart): `POST /api/sessions/restart {id,
    /// force}`. `name` is captured at press time so the result notice names
    /// what the user was looking at, even if the tile's title moved on
    /// while the call was in flight.
    RestartSession {
        id: String,
        name: String,
        force: bool,
    },
    /// `r a`: one restart call per idle/done session of the focused
    /// workspace (the daemon has no workspace filter), aggregated into one
    /// result notice.
    RestartWorkspace { ids: Vec<String> },
    /// `r d`: `POST /api/daemon/restart` — tmux sessions untouched; the SSE
    /// task's existing reconnect handles the dropped connection.
    RestartDaemon,
    /// A plugin action: `POST /api/plugins/{plugin}/actions/{action}/run`
    /// with the UI token. `body` is the run context (focus and trigger).
    RunPluginAction {
        plugin: String,
        action: String,
        body: serde_json::Value,
    },
    /// An action that opens one of the plugin's panes. The pane itself
    /// arrives as an SSE `pane` frame.
    /// `human`: a person's key or click asked for it (so the pane it opens may
    /// take the screen); a plugin's own request opens it unseen.
    OpenPluginPane {
        plugin: String,
        pane: String,
        body: serde_json::Value,
        human: bool,
    },
    /// The user closed a plugin pane: `DELETE /api/plugins/{plugin}/panes/{pane}`.
    ClosePluginPane { plugin: String, pane: String },
}

pub(super) fn send_notice(tx: &EventSender, text: String, ttl_ms: i64) {
    let _ = tx.send(AppEvent::Notice { text, ttl_ms });
}

/// The Dart failure wording: an API-level error carries the daemon's
/// message (`"<prefix>: <message>"`); a transport failure is just the
/// prefix.
pub(super) fn failure_notice(prefix: &str, e: &ApiError) -> String {
    match e {
        ApiError::Status { message, .. } => format!("{prefix}: {message}"),
        ApiError::Transport(_) => prefix.to_owned(),
    }
}

/// The one strip line for `I` (spec tui-hooks-install): the hooks result,
/// then the statusline result. Each side is reported on its own — a failure
/// names which install failed and its error without hiding the other — and
/// an idempotent hooks re-run reads "already installed", never as an error.
/// The statusline success keeps its exact wording.
pub(super) fn install_notice(
    statusline: &Result<(), ApiError>,
    hooks: &Result<HooksInstallResult, ApiError>,
) -> String {
    let hooks = match hooks {
        Ok(result) if result.already_installed => "hooks already installed".to_owned(),
        Ok(_) => "hooks installed".to_owned(),
        Err(e) => failure_notice("hooks install failed", e),
    };
    let statusline = match statusline {
        Ok(()) => INSTALL_STATUSLINE_SUCCESS.to_owned(),
        Err(e) => failure_notice("statusline install failed", e),
    };
    format!("{hooks} · {statusline}")
}

/// p16-restart: the strip line for ONE `r r` restart's response. Every
/// branch names an outcome — a response that restarted nothing still says
/// so, rather than leaving the strip silent about a key the user pressed.
pub(super) fn restart_notice(name: &str, response: &RestartResponse) -> String {
    if let Some(entry) = response.restarted.first() {
        return if entry.resumed {
            format!("{name} restarted")
        } else {
            format!("{name} restarted fresh (no conversation to resume)")
        };
    }
    if let Some(entry) = response.failed.first() {
        return format!("{name}: {}", entry.error);
    }
    if let Some(entry) = response.skipped.first() {
        return format!("{name} skipped — {}", entry.status);
    }
    format!("{name}: the daemon reported no outcome")
}

/// p16-restart: the aggregated `r a` result line. Skips and failures are
/// named only when there are any — the targets were filtered to idle/done
/// before the calls went out, so "skipped 0" is the normal case and would
/// be pure noise; a skip that does appear is a session that went busy
/// between the keypress and the call.
pub(super) fn restart_all_notice(restarted: usize, skipped: usize, failures: &[String]) -> String {
    let mut parts = vec![format!("restarted {restarted}")];
    if skipped > 0 {
        parts.push(format!("skipped {skipped} (working/waiting)"));
    }
    if let Some(first) = failures.first() {
        parts.push(format!("failed {}: {first}", failures.len()));
    }
    parts.join(" · ")
}

/// Run one router effect on a blocking task; results flow back through the
/// channel. `Effect::Quit` is the loop's own concern and never lands here.
pub(super) fn run_effect(effect: Effect, tx: &EventSender, base_url: &str) {
    let tx = tx.clone();
    let base_url = base_url.to_owned();
    tokio::task::spawn_blocking(move || {
        let client = GarageClient::new(Some(base_url));
        match effect {
            Effect::Spawn {
                workspace,
                label,
                worktree,
            } => {
                match client.spawn_session(&workspace, &label, worktree) {
                    Ok(spawned) => {
                        let _ = tx.send(AppEvent::SessionSpawned(spawned.id));
                    }
                    Err(e) => send_notice(&tx, failure_notice("spawn failed", &e), 5000),
                }
                fetch_and_send(&client, &tx);
                let _ = tx.send(AppEvent::SpawnSettled);
            }
            Effect::Restore { id } => {
                match client.restore_session(&id) {
                    Ok(None) => {}
                    Ok(Some(reason)) => send_notice(&tx, format!("restore failed: {reason}"), 5000),
                    Err(e) => send_notice(&tx, failure_notice("restore failed", &e), 5000),
                }
                fetch_and_send(&client, &tx);
                let _ = tx.send(AppEvent::RestoreSettled(id));
            }
            Effect::Close {
                id,
                label,
                meta_only,
            } => {
                match client.delete_session(&id, meta_only) {
                    Ok(Some(record)) => {
                        // Spec tui-worktree-finish: the worktree outlives the
                        // session — hand its record to the finish overlay.
                        send_notice(&tx, format!("closed {label}"), 2000);
                        let _ = tx.send(AppEvent::WorktreeClosed(record));
                    }
                    Ok(None) => send_notice(&tx, format!("closed {label}"), 2000),
                    Err(e) => send_notice(&tx, failure_notice("close failed", &e), 5000),
                }
                fetch_and_send(&client, &tx);
            }
            Effect::Forget { id, label } => {
                match client.forget_session(&id) {
                    Ok(Some(record)) => {
                        send_notice(&tx, format!("deleted {label}"), 2000);
                        let _ = tx.send(AppEvent::WorktreeClosed(record));
                    }
                    Ok(None) => send_notice(&tx, format!("deleted {label}"), 2000),
                    Err(e) => send_notice(&tx, failure_notice("delete failed", &e), 5000),
                }
                fetch_and_send(&client, &tx);
            }
            Effect::RemoveWorkspace { name, kill: false } => {
                match client.remove_workspace(&name, false) {
                    Ok(_) => send_notice(
                        &tx,
                        format!("removed workspace {name} — its sessions keep running"),
                        5000,
                    ),
                    Err(e) => send_notice(&tx, failure_notice("remove failed", &e), 5000),
                }
                fetch_and_send(&client, &tx);
            }
            Effect::RemoveWorkspace { name, kill: true } => {
                match client.remove_workspace(&name, true) {
                    Ok(body) => send_notice(&tx, kill_remove_notice(&name, body.as_ref()), 5000),
                    Err(e) => send_notice(&tx, failure_notice("remove failed", &e), 5000),
                }
                fetch_and_send(&client, &tx);
            }
            Effect::AddWorkspace { name, dir } => match client.put_workspace(&name, &dir) {
                Ok(()) => {
                    // Refetch BEFORE settling so the new group exists when
                    // the settle handler focuses it.
                    fetch_and_send(&client, &tx);
                    let _ = tx.send(AppEvent::WorkspaceAddSettled { name, error: None });
                }
                Err(e) => {
                    let error = match &e {
                        ApiError::Status { message, .. } => message.clone(),
                        ApiError::Transport(_) => "could not register workspace".to_owned(),
                    };
                    let _ = tx.send(AppEvent::WorkspaceAddSettled {
                        name,
                        error: Some(error),
                    });
                }
            },
            Effect::PickDirectory => {
                // Blocks this task (not the loop, not other effects) for as
                // long as the dialog is open.
                let outcome = match client.pick_directory() {
                    Ok(Some(dir)) => PickOutcome::Picked(dir),
                    Ok(None) => PickOutcome::Cancelled,
                    Err(ApiError::Status { status: 501, .. }) => PickOutcome::Unsupported,
                    Err(_) => PickOutcome::Failed,
                };
                let _ = tx.send(AppEvent::PickDirectorySettled(outcome));
            }
            Effect::Install => {
                // Statusline first, then hooks — both always run, so one
                // failing never hides the other's result.
                let statusline = client.install_statusline();
                let hooks = client.install_hooks();
                let _ = tx.send(AppEvent::InstallSettled { statusline, hooks });
            }
            Effect::FinishWorktree { record, action } => {
                let error = match client.finish_worktree(&record, action) {
                    Ok(()) => None,
                    Err(ApiError::Status { status, message }) if message.trim().is_empty() => {
                        Some(format!("{} failed ({status})", action.as_str()))
                    }
                    Err(ApiError::Status { message, .. }) => Some(message),
                    Err(ApiError::Transport(_)) => {
                        Some("could not reach the daemon — try again or keep".to_owned())
                    }
                };
                let _ = tx.send(AppEvent::WorktreeFinishSettled {
                    record,
                    action,
                    error,
                });
            }
            Effect::OpenWindow { id, label } => match crate::ui::window_open::launch(&id) {
                Ok(()) => send_notice(&tx, format!("opened {label} in a new window"), 3000),
                Err(e) => send_notice(&tx, format!("open window failed: {e}"), 5000),
            },
            Effect::RestartSession { id, name, force } => {
                match client.restart_session(&id, force) {
                    Ok(response) => send_notice(&tx, restart_notice(&name, &response), 5000),
                    Err(e) => send_notice(&tx, failure_notice("restart failed", &e), 5000),
                }
                // The pane is new: title, status and context all moved.
                fetch_and_send(&client, &tx);
            }
            Effect::RestartWorkspace { ids } => {
                let mut restarted = 0usize;
                let mut skipped = 0usize;
                let mut failures: Vec<String> = Vec::new();
                // Sequential, one id per call (design.md: the daemon has no
                // workspace filter) — a workspace holds at most a handful of
                // sessions, and one failure must never stop the rest.
                for id in &ids {
                    match client.restart_session(id, false) {
                        Ok(response) => {
                            restarted += response.restarted.len();
                            skipped += response.skipped.len();
                            failures.extend(response.failed.iter().map(|f| f.error.clone()));
                        }
                        Err(e) => failures.push(failure_notice("restart failed", &e)),
                    }
                }
                send_notice(&tx, restart_all_notice(restarted, skipped, &failures), 5000);
                fetch_and_send(&client, &tx);
            }
            Effect::RestartDaemon => match client.restart_daemon() {
                // The successor's pid is the daemon's own bookkeeping; the
                // wall's side of the handoff is already told by the SSE
                // task's `AppEvent::Connection` transitions (the pet boxes
                // on the drop, unboxes on the reconnect), so a success adds
                // no notice of its own.
                Ok(_pid) => {}
                Err(e) => send_notice(&tx, failure_notice("daemon restart failed", &e), 5000),
            },
            Effect::RunPluginAction {
                plugin,
                action,
                body,
            } => {
                if let Err(e) = client.run_plugin_action(&plugin, &action, &body) {
                    send_notice(
                        &tx,
                        failure_notice(&format!("{plugin}: {action} failed"), &e),
                        5000,
                    );
                }
            }
            Effect::OpenPluginPane {
                plugin, pane, body, ..
            } => {
                if let Err(e) = client.open_plugin_pane(&plugin, &pane, &body) {
                    send_notice(
                        &tx,
                        failure_notice(&format!("{plugin}: cannot open {pane}"), &e),
                        5000,
                    );
                }
            }
            Effect::ClosePluginPane { plugin, pane } => {
                if let Err(e) = client.close_plugin_pane(&plugin, &pane) {
                    send_notice(
                        &tx,
                        failure_notice(&format!("{plugin}: cannot close {pane}"), &e),
                        4000,
                    );
                }
            }
            Effect::Quit => unreachable!("Quit is handled by the state loop"),
        }
    });
}
