//! Port of `daemon/src/workspaces.js`: list, register, rename and remove workspaces.
//!
//! Bodies are parsed by hand (`hooks::parse_body`, JS `req.body ?? {}`), never
//! with axum's `Json` extractor, so a missing field is the contract's 400
//! `{error}` and not a plain-text 422.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use garage_protocol::{IdError, RenamedSession};
use serde_json::{json, Map, Value};

use super::hooks::{internal, parse_body};
use crate::backend::tmux::{is_valid_name, GARAGE_PREFIX};
use crate::error::error_response;
use crate::git::resolve_branch;
use crate::state::AppState;

const BAD_NAME: &str = "name must match [a-z0-9-]+";

/// `GET /api/workspaces`: `[{name, dir, branch}]`. The branch is resolved from
/// the registered dir (a workspace has no pane of its own); the git calls run
/// concurrently like the JS `Promise.all`.
pub async fn list(State(state): State<AppState>) -> Response {
    let workspaces = state.registry.list_workspaces().await;
    let mut set = tokio::task::JoinSet::new();
    for (i, w) in workspaces.iter().enumerate() {
        let dir = w.dir.clone().unwrap_or_default();
        set.spawn(async move { (i, resolve_branch(&dir).await) });
    }
    let mut branches = vec![None; workspaces.len()];
    while let Some(Ok((i, b))) = set.join_next().await {
        branches[i] = b;
    }
    let entries: Vec<Value> = workspaces
        .into_iter()
        .zip(branches)
        .map(|(w, branch)| {
            let mut o = Map::new();
            o.insert("name".into(), json!(w.name));
            // JS `{...w}` drops an undefined `dir`.
            if let Some(d) = &w.dir {
                o.insert("dir".into(), json!(d));
            }
            o.insert("branch".into(), json!(branch));
            Value::Object(o)
        })
        .collect();
    Json(entries).into_response()
}

/// `PUT /api/workspaces`: register (or re-point) a workspace.
pub async fn put(State(state): State<AppState>, body: Bytes) -> Response {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let name = body.get("name").and_then(Value::as_str).unwrap_or("");
    if !is_valid_name(name) {
        return error_response(StatusCode::BAD_REQUEST, BAD_NAME);
    }
    let dir = body.get("dir").and_then(Value::as_str);
    let is_dir = match dir {
        Some(d) => tokio::fs::metadata(d)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false),
        None => false,
    };
    if !is_dir {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("dir is not a directory: {}", js_display(body.get("dir"))),
        );
    }
    match state.registry.upsert_workspace(name, dir.unwrap()).await {
        Ok(w) => Json(json!({"name": w.name, "dir": w.dir})).into_response(),
        Err(e) => internal(e.into()),
    }
}

/// How a JS template literal prints a body value: absent is `undefined`.
pub(crate) fn js_display(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) => "null".into(),
        Some(other) => other.to_string(),
    }
}

fn id_error(id: &str, err: &anyhow::Error) -> IdError {
    IdError {
        id: id.to_owned(),
        error: err.to_string(),
    }
}

/// `PATCH /api/workspaces/{name}`: renames the registry key, every live tmux
/// session under it and their resume-metadata keys. Live sessions are renamed
/// first, best-effort with no rollback, so a mid-flight failure leaves the
/// registry consistent with whichever sessions did get renamed.
pub async fn patch(
    State(state): State<AppState>,
    Path(old): Path<String>,
    body: Bytes,
) -> Response {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    if state.registry.get_workspace(&old).await.is_none() {
        return error_response(StatusCode::NOT_FOUND, format!("unknown workspace: {old}"));
    }
    let new = body.get("name").and_then(Value::as_str).unwrap_or("");
    if !is_valid_name(new) {
        return error_response(StatusCode::BAD_REQUEST, BAD_NAME);
    }
    // `new == old` is allowed (JS quirk): it still attempts the tmux renames.
    if new != old && state.registry.get_workspace(new).await.is_some() {
        return error_response(
            StatusCode::CONFLICT,
            format!("workspace already registered: {new}"),
        );
    }

    let old_prefix = format!("{GARAGE_PREFIX}{old}/");
    let new_prefix = format!("{GARAGE_PREFIX}{new}/");
    let live = state.backend.list().await.unwrap_or_default();
    let mut renamed = Vec::new();
    let mut failed = Vec::new();
    for s in live.iter().filter(|s| s.id.starts_with(&old_prefix)) {
        let new_id = format!("{new_prefix}{}", &s.id[old_prefix.len()..]);
        match state.backend.rename(&s.id, &new_id).await {
            Ok(()) => renamed.push(RenamedSession {
                from: s.id.clone(),
                to: new_id,
            }),
            Err(e) => failed.push(id_error(&s.id, &e)),
        }
    }

    let workspace = match state.registry.rename_workspace(&old, new).await {
        Ok(Some(w)) => w,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, format!("unknown workspace: {old}"))
        }
        Err(e) => return internal(e.into()),
    };
    let mut out = Map::new();
    out.insert("name".into(), json!(workspace.name));
    if let Some(d) = &workspace.dir {
        out.insert("dir".into(), json!(d));
    }
    out.insert("renamedSessions".into(), json!(renamed));
    if !failed.is_empty() {
        out.insert("failedSessions".into(), json!(failed));
    }
    Json(Value::Object(out)).into_response()
}

/// `DELETE /api/workspaces/{name}`. By default tmux is never touched: live
/// sessions keep running and reappear as an unregistered group, and only the
/// registry entry and its resume metadata go (204). `?sessions=kill` kills
/// every live `garage/<name>/*` session first (a failed kill is reported, not
/// fatal) and answers 200 `{removed, killedSessions, failedSessions?}`.
/// Worktree directories and branches are deliberately left alone: bulk
/// removal must never silently discard branch work.
pub async fn delete(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if state.registry.get_workspace(&name).await.is_none() {
        return error_response(StatusCode::NOT_FOUND, format!("unknown workspace: {name}"));
    }
    if query.get("sessions").map(String::as_str) != Some("kill") {
        return match state.registry.remove_workspace(&name).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => internal(e.into()),
        };
    }

    let prefix = format!("{GARAGE_PREFIX}{name}/");
    let live = state.backend.list().await.unwrap_or_default();
    let mut killed = Vec::new();
    let mut failed = Vec::new();
    for s in live.iter().filter(|s| s.id.starts_with(&prefix)) {
        match state.backend.kill(&s.id).await {
            Ok(()) => killed.push(s.id.clone()),
            Err(e) => failed.push(id_error(&s.id, &e)),
        }
    }
    if let Err(e) = state.registry.remove_workspace(&name).await {
        return internal(e.into());
    }
    let mut out = Map::new();
    out.insert("removed".into(), json!(name));
    out.insert("killedSessions".into(), json!(killed));
    if !failed.is_empty() {
        out.insert("failedSessions".into(), json!(failed));
    }
    Json(Value::Object(out)).into_response()
}
