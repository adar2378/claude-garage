//! Port of the route in `daemon/src/worktrees.js`: `POST /api/worktrees/finish`.
//! The git work (`create_worktree`, `current_branch`, `finish_worktree`) lives
//! in `crate::git`, where the sessions routes also use it.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use garage_protocol::FinishAction;
use serde_json::{json, Value};

use super::hooks::parse_body;
use crate::error::error_response;
use crate::git::finish_worktree;
use crate::state::AppState;

fn bad(message: &str) -> Response {
    error_response(StatusCode::BAD_REQUEST, message)
}

/// Validation order is part of the contract: record, then its fields, then the
/// action, and only then (inside `finish_worktree`) the `garage/` branch guard.
pub async fn finish(State(state): State<AppState>, body: Bytes) -> Response {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let Some(worktree) = body.get("worktree").filter(|w| w.is_object() || w.is_array()) else {
        return bad("worktree record required");
    };
    let field = |k: &str| worktree.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let (Some(path), Some(branch), Some(repo_dir)) = (field("path"), field("branch"), field("repoDir")) else {
        return bad("worktree record requires path, branch, and repoDir");
    };
    let action = match body.get("action").and_then(Value::as_str) {
        Some("merge") => FinishAction::Merge,
        Some("discard") => FinishAction::Discard,
        Some("keep") => FinishAction::Keep,
        _ => return bad("action must be one of merge, discard, keep"),
    };

    match finish_worktree(&state.config.garage_dir, repo_dir, path, branch, action).await {
        Ok(result) => Json(result).into_response(),
        Err(e) => {
            let mut out = json!({"error": e.message});
            if let Some(stderr) = e.stderr.filter(|s| !s.is_empty()) {
                out["stderr"] = json!(stderr);
            }
            (StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), Json(out)).into_response()
        }
    }
}
