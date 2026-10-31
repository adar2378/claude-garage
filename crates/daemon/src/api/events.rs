//! Port of `daemon/src/events.js`: `GET /api/events` Server-Sent Events fed by the event bus.
//!
//! Frames: `: connected` first, `event: status` (`{id, status, since}`) on
//! every transition, `event: sessions` (`{}`) when the session set or a pane
//! title changes (and after a restart), `: keepalive` every 15 s.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream};
use serde_json::json;
use tokio::sync::broadcast::{error::RecvError, Receiver};
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
