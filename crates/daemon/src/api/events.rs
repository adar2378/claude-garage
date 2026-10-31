//! Port of `daemon/src/events.js`: `GET /api/events` Server-Sent Events fed by the event bus.
//!
//! Frames: `: connected` first, `event: status` (`{id, status, since}`) on
//! every transition, `event: sessions` (`{}`) when the session set or a pane
//! title changes (and after a restart), `: keepalive` every 15 s.
//!
//! Plugin host frames (p24, additive; older clients ignore them):
//! `session.created`, `session.closed` (with `reason`), `session.turn`, `session.forgotten`,
//! `worktree.created`, `workspace.added`, `workspace.removed` carry the same
//! payload a plugin event hook gets; `widgets` (`{plugin, rev}`), `plugins`
//! (`{}`), `plugin_run_failed`, `plugin_notice`, `ui_action` (a plugin asks the
//! TUI to run an action) and `pane` (a plugin pane opened or closed) serve the TUI.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream};
use serde_json::json;
use tokio::sync::broadcast::{Receiver, error::RecvError};
use tokio::sync::watch;

use crate::bus::DaemonEvent;
use crate::state::AppState;

pub const KEEPALIVE: Duration = Duration::from_secs(15);

fn frame(event: &DaemonEvent) -> Event {
    match event {
        DaemonEvent::StatusTransition { id, to, since, .. } => Event::default()
            .event("status")
            .data(json!({"id": id, "status": to, "since": since}).to_string()),
        DaemonEvent::SessionsChanged => Event::default().event("sessions").data("{}"),
        DaemonEvent::PluginRunFailed {
            plugin,
            action,
            reason,
        } => Event::default()
            .event("plugin_run_failed")
            .data(json!({"plugin": plugin, "action": action, "reason": reason}).to_string()),
        DaemonEvent::PluginNotice {
            plugin,
            message,
            ttl_ms,
        } => Event::default()
            .event("plugin_notice")
            .data(json!({"plugin": plugin, "message": message, "ttl_ms": ttl_ms}).to_string()),
        DaemonEvent::WidgetsChanged { plugin, rev } => Event::default()
            .event("widgets")
            .data(json!({"plugin": plugin, "rev": rev}).to_string()),
        DaemonEvent::PluginsChanged => Event::default().event("plugins").data("{}"),
        DaemonEvent::UiAction {
            plugin,
            action,
            args,
            target,
        } => Event::default().event("ui_action").data(
            serde_json::to_string(&garage_protocol::plugins::UiActionFrame {
                plugin: plugin.clone(),
                action: action.clone(),
                args: args.clone(),
                target: target.clone(),
            })
            .unwrap_or_else(|_| "{}".into()),
        ),
        DaemonEvent::PaneChanged {
            plugin,
            pane,
            session,
            open,
        } => Event::default().event("pane").data(
            json!({"plugin": plugin, "pane": pane, "session": session, "open": open}).to_string(),
        ),
        // The typed events keep their plugin-visible names (`session.closed`, ...).
        typed => match typed.plugin_event() {
            Some((name, data)) => Event::default().event(name).data(data.to_string()),
            None => Event::default().comment("unknown"),
        },
    }
}

/// `: connected`, then one frame per bus event until the bus closes or the
/// daemon shuts down (an open SSE body would otherwise hold graceful shutdown
/// forever). A lagging subscriber lost events it cannot replay, so it gets a
/// `sessions` frame and the UI refetches.
fn events(
    rx: Receiver<DaemonEvent>,
    shutdown: watch::Receiver<bool>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold((rx, false), move |(mut rx, greeted)| {
        let mut stop = shutdown.clone();
        async move {
            if !greeted {
                return Some((Ok(Event::default().comment("connected")), (rx, true)));
            }
            let stopped = async {
                if stop.wait_for(|v| *v).await.is_err() {
                    std::future::pending::<()>().await;
                }
            };
            tokio::select! {
                _ = stopped => None,
                r = rx.recv() => match r {
                    Ok(ev) => Some((Ok(frame(&ev)), (rx, true))),
                    Err(RecvError::Lagged(_)) => Some((Ok(frame(&DaemonEvent::SessionsChanged)), (rx, true))),
                    Err(RecvError::Closed) => None,
                },
            }
        }
    })
}

pub async fn stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe before returning so nothing published after the response
    // starts can be missed.
    Sse::new(events(state.bus.subscribe(), state.shutdown_signal()))
        .keep_alive(KeepAlive::new().interval(KEEPALIVE).text("keepalive"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventBus;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    async fn next_text(body: &mut axum::body::Body) -> String {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("frame in time");
        String::from_utf8(frame.unwrap().unwrap().into_data().unwrap().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn a_lagged_subscriber_gets_a_sessions_frame_so_the_ui_refetches() {
        let bus = EventBus::new(2);
        let rx = bus.subscribe();
        let (_tx, sd) = watch::channel(false);
        let mut body = Sse::new(events(rx, sd)).into_response().into_body();
        assert_eq!(next_text(&mut body).await, ": connected\n\n");
        // Five events into a two-slot channel: the subscriber misses some.
        for _ in 0..5 {
            bus.publish(DaemonEvent::SessionsChanged);
        }
        assert_eq!(next_text(&mut body).await, "event: sessions\ndata: {}\n\n");
    }

    #[tokio::test]
    async fn plugin_host_events_become_additive_frames_older_clients_ignore() {
        use crate::closing::CloseReason;
        let bus = EventBus::default();
        let (_tx, sd) = watch::channel(false);
        let mut body = Sse::new(events(bus.subscribe(), sd))
            .into_response()
            .into_body();
        assert_eq!(next_text(&mut body).await, ": connected\n\n");
        let cases = [
            (
                DaemonEvent::WidgetsChanged {
                    plugin: "usage-bar".into(),
                    rev: 42,
                },
                "widgets",
                json!({"plugin": "usage-bar", "rev": 42}),
            ),
            (DaemonEvent::PluginsChanged, "plugins", json!({})),
            (
                DaemonEvent::PluginRunFailed {
                    plugin: "p".into(),
                    action: "p:go".into(),
                    reason: "exited with code 3".into(),
                },
                "plugin_run_failed",
                json!({"plugin": "p", "action": "p:go", "reason": "exited with code 3"}),
            ),
            (
                DaemonEvent::PluginNotice {
                    plugin: "p".into(),
                    message: "hi".into(),
                    ttl_ms: Some(3000),
                },
                "plugin_notice",
                json!({"plugin": "p", "message": "hi", "ttl_ms": 3000}),
            ),
            (
                DaemonEvent::UiAction {
                    plugin: "p".into(),
                    action: "session.close".into(),
                    args: Some(json!({"k": 1})),
                    target: garage_protocol::plugins::UiTarget {
                        session: Some("garage/w/a".into()),
                        workspace: None,
                    },
                },
                "ui_action",
                json!({"plugin": "p", "action": "session.close", "args": {"k": 1}, "target": {"session": "garage/w/a"}}),
            ),
            (
                DaemonEvent::PaneChanged {
                    plugin: "p".into(),
                    pane: "d".into(),
                    session: "plugin/p/d".into(),
                    open: true,
                },
                "pane",
                json!({"plugin": "p", "pane": "d", "session": "plugin/p/d", "open": true}),
            ),
            (
                DaemonEvent::SessionClosed {
                    id: "garage/w/a".into(),
                    dir: "/d".into(),
                    workspace: "w".into(),
                    reason: CloseReason::DaemonRestart,
                    last_status: Some("idle".into()),
                },
                "session.closed",
                json!({"id": "garage/w/a", "dir": "/d", "workspace": "w", "reason": "daemon-restart", "last_status": "idle"}),
            ),
            (
                DaemonEvent::SessionTurn {
                    id: "garage/w/a".into(),
                    workspace: "w".into(),
                    at: 5,
                    transcript_offset: None,
                    row: Some(42),
                    history_limit: Some(2000),
                },
                "session.turn",
                json!({"id": "garage/w/a", "workspace": "w", "at": 5, "transcript_offset": null, "row": 42, "history_limit": 2000}),
            ),
            (
                DaemonEvent::SessionForgotten {
                    id: "garage/w/a".into(),
                    workspace: "w".into(),
                },
                "session.forgotten",
                json!({"id": "garage/w/a", "workspace": "w"}),
            ),
            (
                DaemonEvent::WorkspaceAdded { name: "w".into() },
                "workspace.added",
                json!({"name": "w"}),
            ),
        ];
        for (ev, name, data) in cases {
            bus.publish(ev);
            let text = next_text(&mut body).await;
            let (head, tail) = text.split_once('\n').unwrap();
            assert_eq!(head, format!("event: {name}"));
            let payload: serde_json::Value =
                serde_json::from_str(tail.trim().strip_prefix("data: ").unwrap()).unwrap();
            assert_eq!(payload, data, "{name}");
        }
    }

    #[tokio::test]
    async fn the_stream_ends_when_shutdown_is_signalled() {
        let bus = EventBus::default();
        let (tx, sd) = watch::channel(false);
        let mut body = Sse::new(events(bus.subscribe(), sd))
            .into_response()
            .into_body();
        assert_eq!(next_text(&mut body).await, ": connected\n\n");
        tx.send(true).unwrap();
        let end = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("ends in time");
        assert!(end.is_none(), "no more frames after shutdown");
    }
}
