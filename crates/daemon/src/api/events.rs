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

use crate::bus::DaemonEvent;
use crate::state::AppState;

pub const KEEPALIVE: Duration = Duration::from_secs(15);

fn frame(event: &DaemonEvent) -> Event {
    match event {
        DaemonEvent::StatusTransition { id, to, since, .. } => {
            Event::default().event("status").data(json!({"id": id, "status": to, "since": since}).to_string())
        }
        DaemonEvent::SessionsChanged => Event::default().event("sessions").data("{}"),
    }
}

/// `: connected`, then one frame per bus event until the bus closes. A lagging
/// subscriber skips what it missed (the UI refetches on the next event).
fn events(rx: Receiver<DaemonEvent>) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold((rx, false), |(mut rx, greeted)| async move {
        if !greeted {
            return Some((Ok(Event::default().comment("connected")), (rx, true)));
        }
        loop {
            match rx.recv().await {
                Ok(ev) => return Some((Ok(frame(&ev)), (rx, true))),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    })
}

pub async fn stream(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe before returning so nothing published after the response
    // starts can be missed.
    Sse::new(events(state.bus.subscribe())).keep_alive(KeepAlive::new().interval(KEEPALIVE).text("keepalive"))
}
