//! Background tasks feeding the one event channel: terminal input, the
//! periodic fetch and usage polls, SSE, and the signal watcher (moved out of
//! `runtime/mod.rs`, p22 task 1.2).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use std::cell::RefCell;

use serde_json::Value;

use crate::plugins::revs::RevTracker;

use crate::api::client::GarageClient;
use crate::api::sse::SseClient;

use super::{AppEvent, EventSender, USAGE_POLL_INTERVAL, send_notice};

/// Refetch both listings and feed them into the channel (the Dart
/// `_refetchSessions`: workspaces too, so worktree flags never derive from a
/// stale registered dir).
pub(super) fn fetch_and_send(client: &GarageClient, tx: &EventSender) {
    if let Ok(workspaces) = client.fetch_workspaces() {
        let _ = tx.send(AppEvent::Workspaces(workspaces));
    }
    if let Ok(sessions) = client.fetch_sessions() {
        let _ = tx.send(AppEvent::Sessions(sessions));
    }
}

/// Fetch the plugin listing and every plugin's widgets (startup, `plugins`
/// frame, reconnect). The listing goes first: widget content of a plugin the
/// listing does not call active is ignored.
fn fetch_plugins_full(client: &GarageClient, tx: &EventSender, revs: &RefCell<RevTracker>) {
    let Ok(list) = client.fetch_plugins() else { return };
    let _ = tx.send(AppEvent::Plugins(list));
    if let Ok(snapshot) = client.fetch_plugin_widgets(None) {
        revs.borrow_mut().record_all(snapshot.plugins.iter().map(|(id, w)| (id.clone(), w.rev)));
        let _ = tx.send(AppEvent::PluginWidgets { snapshot, full: true });
    }
}

/// The plugin-host SSE frames. Returns false for a frame it does not know.
fn plugin_frame(event: &crate::api::sse::SseEvent, client: &GarageClient, tx: &EventSender, revs: &RefCell<RevTracker>) -> bool {
    let data: Value = serde_json::from_str(&event.data).unwrap_or(Value::Null);
    let text = |k: &str| data.get(k).and_then(Value::as_str).map(str::to_owned);
    match event.event.as_str() {
        "plugins" => {
            revs.borrow_mut().reset();
            fetch_plugins_full(client, tx, revs);
        }
        "widgets" => {
            let (Some(plugin), Some(rev)) = (text("plugin"), data.get("rev").and_then(Value::as_u64)) else { return true };
            let news = revs.borrow().should_fetch(&plugin, rev);
            if news {
                // The rev counts as held only once its content arrived; a failed
                // fetch leaves it unseen so the next frame asks again.
                if let Ok(snapshot) = client.fetch_plugin_widgets(Some(&plugin)) {
                    revs.borrow_mut().fetched(&plugin, snapshot.plugins.get(&plugin).map_or(rev, |w| w.rev));
                    let _ = tx.send(AppEvent::PluginWidgets { snapshot, full: false });
                }
            }
        }
        "plugin_notice" => {
            if let (Some(plugin), Some(message)) = (text("plugin"), text("message")) {
                let _ = tx.send(AppEvent::PluginNotice(format!("{plugin}: {message}")));
            }
        }
        "plugin_run_failed" => {
            if let (Some(plugin), Some(action)) = (text("plugin"), text("action")) {
                let reason = text("reason").unwrap_or_default();
                let _ = tx.send(AppEvent::PluginNotice(format!("{plugin}: {action} failed ({reason}) — see `claude-garage plugin logs {plugin}`")));
            }
        }
        "ui_action" => {
            if let Ok(frame) = serde_json::from_value(data) {
                let _ = tx.send(AppEvent::UiAction(frame));
            }
        }
        "pane" => {
            let Some(session) = text("session") else { return true };
            if data.get("open").and_then(Value::as_bool) == Some(true) {
                if let Some(pane) = client.fetch_plugin_panes().ok().and_then(|r| r.panes.into_iter().find(|p| p.session == session)) {
                    let _ = tx.send(AppEvent::PaneOpened(pane));
                }
            } else {
                let _ = tx.send(AppEvent::PaneClosed { session });
            }
        }
        _ => return false,
    }
    true
}

/// Terminal input → channel. Poll-based so the `stop` flag is honored
/// promptly at shutdown.
pub(super) fn spawn_input_task(rt: &tokio::runtime::Runtime, tx: EventSender, stop: Arc<AtomicBool>) {
    rt.spawn_blocking(move || {
        while !stop.load(Ordering::Relaxed) {
            match crossterm::event::poll(Duration::from_millis(100)) {
                Ok(true) => match crossterm::event::read() {
                    Ok(ev) => {
                        if tx.send(AppEvent::Term(ev)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });
}

/// Initial workspaces + sessions fetch, plus the visible=true report (the
/// full visibility heartbeat is wave 4's escalation module).
pub(super) fn spawn_fetch_task(
    rt: &tokio::runtime::Runtime,
    tx: EventSender,
    base_url: String,
    client_id: String,
) {
    rt.spawn_blocking(move || {
        let client = GarageClient::new(Some(base_url));
        let _ = client.post_visibility(&client_id, true);
        match client.fetch_workspaces() {
            Ok(workspaces) => {
                let _ = tx.send(AppEvent::Workspaces(workspaces));
            }
            Err(e) => send_notice(&tx, format!("workspaces fetch failed: {e}"), 5000),
        }
        match client.fetch_sessions() {
            Ok(sessions) => {
                let _ = tx.send(AppEvent::Sessions(sessions));
            }
            Err(e) => send_notice(&tx, format!("sessions fetch failed: {e}"), 5000),
        }
    });
}

/// `GET /api/usage`: once at startup, then every [`USAGE_POLL_INTERVAL`]
/// (spec tui-context-meters "Strip usage chip") — no SSE push exists for
/// account-wide usage, so polling is the only source. Stops promptly on
/// `stop` rather than sleeping through the full interval at shutdown.
pub(super) fn spawn_usage_task(
    rt: &tokio::runtime::Runtime,
    tx: EventSender,
    base_url: String,
    stop: Arc<AtomicBool>,
) {
    rt.spawn(async move {
        while !stop.load(Ordering::Relaxed) {
            let base_url = base_url.clone();
            let tx = tx.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let client = GarageClient::new(Some(base_url));
                if let Ok(usage) = client.fetch_usage() {
                    let _ = tx.send(AppEvent::Usage(usage));
                }
            })
            .await;
            tokio::time::sleep(USAGE_POLL_INTERVAL).await;
        }
    });
}

/// SSE subscription task: `status` events flow straight into the channel; a
/// `sessions` ping (and the poll fallback while disconnected) triggers a
/// refetch whose result flows into the same channel.
pub(super) fn spawn_sse_task(
    rt: &tokio::runtime::Runtime,
    tx: EventSender,
    base_url: String,
    stop: Arc<AtomicBool>,
) {
    rt.spawn_blocking(move || {
        let base_url_for_plugins = base_url.clone();
        let client = GarageClient::new(Some(base_url.clone()));
        let mut sse = SseClient::new(base_url);
        let refetch = {
            let tx = tx.clone();
            move |client: &GarageClient| {
                if let Ok(sessions) = client.fetch_sessions() {
                    let _ = tx.send(AppEvent::Sessions(sessions));
                }
            }
        };
        let revs = std::rc::Rc::new(RefCell::new(RevTracker::default()));
        let on_event = {
            let tx = tx.clone();
            let client = &client;
            let refetch = refetch.clone();
            let revs = revs.clone();
            move |event: crate::api::sse::SseEvent| match event.event.as_str() {
                "status" => {
                    if let Ok(v) = serde_json::from_str::<Value>(&event.data) {
                        if let Some(id) = v.get("id").and_then(Value::as_str) {
                            let status = v
                                .get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("idle")
                                .to_owned();
                            let needs_input = status == "needs-input";
                            let _ = tx.send(AppEvent::Status {
                                id: id.to_owned(),
                                status,
                                since: v.get("since").and_then(Value::as_i64),
                            });
                            // The status event carries no notification text;
                            // a needs-input transition's `message` only
                            // exists on the listing, so pull it — the triage
                            // queue must show the daemon's question (spec
                            // tui-triage "Question text shown").
                            if needs_input {
                                refetch(client);
                            }
                        }
                    }
                }
                "sessions" => refetch(client),
                _ => {
                    plugin_frame(&event, client, &tx, &revs);
                }
            }
        };
        let on_poll = {
            let client = &client;
            move || refetch(client)
        };
        // `AppEvent::Connection` transitions only (spec tui-pit-pet: the
        // pet boxes on an observed drop, not on every internal reconnect
        // tick) — `last_live` collapses `SseReconnectMachine`'s repeated
        // connect/disconnect calls (one per attempt/poll cycle) down to an
        // edge so the state loop isn't spammed with redundant events.
        let last_live = std::cell::Cell::new(None::<bool>);
        let plugin_client = GarageClient::new(Some(base_url_for_plugins));
        let on_connection = move |live: bool| {
            if last_live.get() != Some(live) {
                last_live.set(Some(live));
                let _ = tx.send(AppEvent::Connection(live));
                // Events were missed while the stream was down (and on the
                // very first connect there is nothing yet): take the full
                // plugin picture.
                if live {
                    revs.borrow_mut().reset();
                    fetch_plugins_full(&plugin_client, &tx, &revs);
                }
            }
        };
        sse.run(stop, on_event, on_poll, on_connection);
    });
}

/// SIGTERM → orderly quit (emergency kill contract: the terminal is restored
/// and tiles detach before masters close).
pub(super) fn spawn_signal_task(rt: &tokio::runtime::Runtime, tx: EventSender) {
    rt.spawn(async move {
        #[cfg(unix)]
        {
            if let Ok(mut sigterm) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            {
                sigterm.recv().await;
                let _ = tx.send(AppEvent::Quit);
            }
        }
        // Windows has no SIGTERM; the console's close/break event is the
        // nearest thing.
        #[cfg(windows)]
        {
            if let Ok(mut close) = tokio::signal::windows::ctrl_close() {
                close.recv().await;
                let _ = tx.send(AppEvent::Quit);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::sse::SseEvent;

    #[test]
    fn a_widgets_frame_whose_fetch_failed_is_asked_for_again_on_the_next_frame() {
        // Nothing listens on port 1: the fetch fails.
        let client = GarageClient::new(Some("http://127.0.0.1:1".into()));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let revs = RefCell::new(RevTracker::default());
        let frame = SseEvent { event: "widgets".into(), data: r#"{"plugin":"bt","rev":3}"#.into() };
        assert!(plugin_frame(&frame, &client, &tx, &revs));
        assert!(rx.try_recv().is_err(), "nothing was fetched, so nothing is applied");
        assert!(revs.borrow().should_fetch("bt", 3), "the rev is not remembered after a failed fetch");
        // Once the content really arrived the same rev is news no more.
        revs.borrow_mut().fetched("bt", 3);
        assert!(!revs.borrow().should_fetch("bt", 3));
    }
}
