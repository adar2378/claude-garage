//! HTTP routes. Every route in the `daemon-api` spec is registered; all but
//! `/api/health` answer 501 until their module is ported.

pub mod attach;
pub mod daemon_restart;
pub mod editor;
pub mod events;
pub mod hooks;
pub mod picker;
pub mod sessions;
pub mod statusline;
pub mod visibility;
pub mod workspaces;
pub mod worktrees;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use garage_protocol::Health;

use crate::error::error_response;
use crate::state::AppState;

pub async fn health(State(_state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: "ok".into(),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        pid: std::process::id(),
    })
}

/// Unknown routes (including the removed `GET /api/diff/:workspace`) are 404.
pub async fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "not found")
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/events", get(events::stream))
        .route("/api/attach", get(attach::get))
        .route("/api/sessions", get(sessions::list).post(sessions::create))
        .route("/api/sessions/restore", post(sessions::restore))
        .route("/api/sessions/restart", post(sessions::restart))
        // The id contains slashes, so capture the whole tail.
        .route("/api/sessions/{*id}", delete(sessions::remove))
        .route("/api/workspaces", get(workspaces::list).put(workspaces::put))
        .route("/api/workspaces/{name}", patch(workspaces::patch).delete(workspaces::delete))
        .route("/api/hooks/claude", post(hooks::claude))
        .route("/api/hooks/snippet", get(hooks::snippet))
        .route("/api/hooks/install", post(hooks::install))
        .route("/api/statusline/claude", post(statusline::claude))
        .route("/api/statusline/snippet", get(statusline::snippet))
        .route("/api/statusline/install", post(statusline::install))
        .route("/api/usage", get(statusline::usage))
        .route("/api/ui/visibility", post(visibility::post))
        .route("/api/daemon/restart", post(daemon_restart::restart))
        .route("/api/open-editor", post(editor::open))
        .route("/api/pick-directory", post(picker::pick))
        .route("/api/worktrees/finish", post(worktrees::finish))
        .fallback(not_found)
}
