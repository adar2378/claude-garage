//! Background tasks feeding the one event channel: terminal input, the
//! periodic fetch and usage polls, SSE, and the signal watcher (moved out of
//! `runtime/mod.rs`, p22 task 1.2).

use super::*;

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
        let on_event = {
            let tx = tx.clone();
            let client = &client;
            let refetch = refetch.clone();
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
                _ => {}
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
        let on_connection = move |live: bool| {
            if last_live.get() != Some(live) {
                last_live.set(Some(live));
                let _ = tx.send(AppEvent::Connection(live));
            }
        };
        sse.run(stop, on_event, on_poll, on_connection);
    });
}

/// SIGTERM → orderly quit (emergency kill contract: the terminal is restored
/// and tiles detach before masters close).
pub(super) fn spawn_signal_task(rt: &tokio::runtime::Runtime, tx: EventSender) {
    rt.spawn(async move {
        if let Ok(mut sigterm) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sigterm.recv().await;
            let _ = tx.send(AppEvent::Quit);
        }
    });
}
