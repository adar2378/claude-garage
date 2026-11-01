//! Port of the route half of `daemon/src/notify.js`: `POST /api/ui/visibility`
//! and the store behind the "is a UI in view?" rule (plugin runs read it as `ui.visible`).
//!
//! The TUI posts here periodically while it is in the foreground. A visible
//! client exists when any entry has `visible` and was seen within `TTL`. Rides
//! the normal Origin allowlist (the TUI sends no Origin).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::hooks::parse_body;
use crate::error::error_response;
use crate::state::AppState;

pub const TTL: Duration = Duration::from_secs(45);

/// `clientId -> (visible, last seen)`.
#[derive(Clone, Default)]
pub struct Visibility {
    clients: Arc<Mutex<HashMap<String, (bool, Instant)>>>,
}

impl Visibility {
    pub fn record(&self, client_id: &str, visible: bool) {
        self.record_at(client_id, visible, Instant::now());
    }

    pub fn record_at(&self, client_id: &str, visible: bool, now: Instant) {
        let mut clients = self.clients.lock().unwrap();
        // Expired entries can never count again; dropping them is unobservable.
        clients.retain(|_, (_, seen)| now.saturating_duration_since(*seen) <= TTL);
        clients.insert(client_id.to_owned(), (visible, now));
    }

    pub fn any_visible(&self) -> bool {
        self.any_visible_at(Instant::now())
    }

    pub fn any_visible_at(&self, now: Instant) -> bool {
        self.clients
            .lock()
            .unwrap()
            .values()
            .any(|(visible, seen)| *visible && now.saturating_duration_since(*seen) <= TTL)
    }
}

pub async fn post(State(state): State<AppState>, body: Bytes) -> Response {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let id = body
        .get("clientId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let visible = body.get("visible").and_then(Value::as_bool);
    let (Some(id), Some(visible)) = (id, visible) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "clientId (string) and visible (boolean) are required",
        );
    };
    state.visibility.record(id, visible);
    Json(json!({"ok": true})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_client_counts_for_45_seconds_and_hidden_never_does() {
        let v = Visibility::default();
        let t0 = Instant::now();
        assert!(!v.any_visible_at(t0));
        v.record_at("tui", true, t0);
        assert!(v.any_visible_at(t0 + Duration::from_secs(10)));
        assert!(
            v.any_visible_at(t0 + TTL),
            "exactly at the TTL still counts (JS `<=`)"
        );
        assert!(!v.any_visible_at(t0 + TTL + Duration::from_millis(1)));
        v.record_at("tui", false, t0 + Duration::from_secs(20));
        assert!(
            !v.any_visible_at(t0 + Duration::from_secs(21)),
            "latest report wins"
        );
    }

    #[test]
    fn any_one_fresh_visible_client_among_stale_and_hidden_ones_suffices() {
        let v = Visibility::default();
        let t0 = Instant::now();
        v.record_at("stale", true, t0);
        v.record_at("hidden", false, t0 + Duration::from_secs(50));
        assert!(!v.any_visible_at(t0 + Duration::from_secs(60)));
        v.record_at("fresh", true, t0 + Duration::from_secs(55));
        assert!(v.any_visible_at(t0 + Duration::from_secs(60)));
    }
}
