//! Port of the route in `daemon/src/picker.js`: `POST /api/pick-directory` via `os::Os::pick_directory`.
//!
//! Browsers never expose absolute filesystem paths, so the picker runs
//! daemon-side. Where the OS has none the answer is 501, as on non-macOS today.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::error::error_response;
use crate::os::{strip_trailing_slash, OsError};
use crate::state::AppState;

pub async fn pick(State(state): State<AppState>) -> Response {
    match state.os.pick_directory().await {
        Ok(Some(dir)) => {
            Json(json!({"dir": strip_trailing_slash(&dir.to_string_lossy())})).into_response()
        }
        Ok(None) => Json(json!({"cancelled": true})).into_response(),
        Err(OsError::Unsupported) => error_response(
            StatusCode::NOT_IMPLEMENTED,
            "no folder picker on this system (macOS, Windows, or Linux with zenity or kdialog)",
        ),
        Err(OsError::Failed(m)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, m),
        Err(OsError::NotFound) => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "folder picker failed")
        }
    }
}
