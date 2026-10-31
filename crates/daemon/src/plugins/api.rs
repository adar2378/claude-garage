//! Plugin-facing and plugin-host routes (design section 8). Plugin routes
//! require the plugin's bearer token and check its declared permissions
//! (soft: a mistake catcher, not a boundary). `POST /api/sessions/input`
//! additionally REQUIRES a token, since it types into sessions.
//!
//! Routes only the TUI (or CLI) may call need the per-install UI token
//! (`<GARAGE_DIR>/ui-token`, `Authorization: Bearer ...`): `plugins/reload`,
//! `plugins/ui/slots`, and running another plugin's action. A plugin's own
//! token may run that plugin's own actions and nothing else here.

use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::{AUTHORIZATION, RETRY_AFTER};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use garage_plugin::manifest::Permission;
use garage_protocol::widgets::{check_body_len, click_actions, limits, validate_put, validate_slot_geometry, WidgetPut};
use serde_json::{json, Value};

use super::{PluginState, RunError};
use crate::api::hooks::parse_body;
use crate::backend::tmux::GARAGE_PREFIX;
use crate::error::error_response;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/plugin/features", get(features))
        .route("/api/plugins/reload", post(reload))
        .route("/api/plugins/ui/slots", put(put_ui_slots))
        .route("/api/plugins/ui/challenge", post(ui_challenge))
        .route("/api/plugins/{id}", axum::routing::delete(remove_plugin))
        .route("/api/plugins/{id}/registration", put(register_plugin))
        .route("/api/plugins/{id}/enable", post(enable_plugin))
        .route("/api/plugins/{id}/disable", post(disable_plugin))
        .route("/api/plugins/{id}/bind", post(bind_plugin))
        .route("/api/plugins/{id}/widgets/{widget}", put(put_widget).delete(delete_widget))
        .route("/api/plugins/{id}/actions/{action}/run", post(run_action))
        .route("/api/sessions/input", post(session_input))
        .route("/api/workspaces/{name}/closed", get(closed))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    // An empty bearer is no credential at all.
    headers.get(AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ").map(str::trim).filter(|t| !t.is_empty())
}

/// A request's plugin, checked for the permission. 401 for a missing or
/// unknown token, 403 for a disabled plugin or a missing permission.
fn authed(st: &AppState, headers: &HeaderMap, perm: Permission) -> Result<PluginState, Box<Response>> {
    let Some(token) = bearer(headers) else {
        return Err(Box::new(error_response(StatusCode::UNAUTHORIZED, "missing plugin token (Authorization: Bearer $GARAGE_PLUGIN_TOKEN)")));
    };
    let Some(plugin) = st.plugins.authenticate(token) else {
        return Err(Box::new(error_response(StatusCode::UNAUTHORIZED, "invalid plugin token")));
    };
    let id = &plugin.entry.id;
    if !plugin.active() {
        return Err(Box::new(error_response(StatusCode::FORBIDDEN, format!("plugin {id} is disabled"))));
    }
    if !plugin.manifest().is_some_and(|m| m.has_permission(perm)) {
        return Err(Box::new(error_response(StatusCode::FORBIDDEN, format!("plugin {id} lacks permission {}", perm.as_str()))));
    }
    Ok(plugin)
}

/// Like [`authed`], and the token must belong to the plugin in the path.
fn authed_for(st: &AppState, headers: &HeaderMap, id: &str, perm: Permission) -> Result<PluginState, Box<Response>> {
    let plugin = authed(st, headers, perm)?;
    if plugin.entry.id != id {
        return Err(Box::new(error_response(StatusCode::FORBIDDEN, format!("this token belongs to plugin {}, not {id}", plugin.entry.id))));
    }
    Ok(plugin)
}

/// The caller must present the UI token.
fn require_ui(st: &AppState, headers: &HeaderMap) -> Result<(), Box<Response>> {
    match bearer(headers) {
        None => Err(Box::new(error_response(StatusCode::UNAUTHORIZED, "missing UI token (Authorization: Bearer <contents of GARAGE_DIR/ui-token>)"))),
        Some(t) if st.plugins.is_ui_token(t) => Ok(()),
        Some(_) => Err(Box::new(error_response(StatusCode::FORBIDDEN, "this route needs the UI token; a plugin token is not enough"))),
    }
}

/// Proves this is the real daemon before a client sends it the UI token:
/// `{"nonce": "<hex>"}` in, `{"proof": HMAC-SHA256(ui_token, ...)}` out. Needs
/// no credentials and reveals nothing but the keyed digest of the nonce.
async fn ui_challenge(State(st): State<AppState>, body: Bytes) -> Response {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let Some(nonce) = body.get("nonce").and_then(Value::as_str).filter(|n| garage_protocol::auth::nonce_ok(n)) else {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "nonce must be 16 to 128 lowercase hex digits");
    };
    match st.plugins.ui_proof(st.config.port, nonce) {
        Some(proof) => Json(json!({"proof": proof})).into_response(),
        None => error_response(StatusCode::SERVICE_UNAVAILABLE, "this daemon has no UI token"),
    }
}

// ---- registry edits for the CLI lane (UI token) ----
//
// The daemon owns `plugins.json`; while it is up the CLI calls these instead
// of editing the file. Each is one locked read-modify-write followed by a
// reload, which also runs the teardown sequence when a plugin stops being
// active.

fn admin_error(e: String) -> Response {
    let status = if e.starts_with("no plugin") {
        StatusCode::NOT_FOUND
    } else if e.contains("bundled") {
        StatusCode::CONFLICT
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };
    error_response(status, e)
}

fn reloaded(st: &AppState) -> Response {
    match st.plugins.reload() {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => error_response(StatusCode::CONFLICT, e),
    }
}

async fn register_plugin(State(st): State<AppState>, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = require_ui(&st, &headers) {
        return *r;
    }
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let source = match body.get("source").cloned().map(serde_json::from_value::<super::layout::Source>) {
        Some(Ok(s)) => s,
        _ => return error_response(StatusCode::UNPROCESSABLE_ENTITY, "source must be {kind: github|link, ...}"),
    };
    let Some(granted) = body
        .get("grantedPermissions")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().map(|v| v.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>())
    else {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "grantedPermissions must be an array of strings");
    };
    if let Some(bad) = granted.iter().find(|p| !garage_plugin::features::PERMISSIONS.contains(&p.as_str())) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, format!("unknown permission {bad:?}"));
    }
    let reg = super::admin::Registration {
        id,
        source,
        granted_permissions: granted,
        manifest_sha256: body.get("manifestSha256").and_then(Value::as_str).map(str::to_owned),
        installed_at: body.get("installedAt").and_then(Value::as_i64),
    };
    match super::admin::register(st.plugins.paths(), &reg) {
        Ok(()) => reloaded(&st),
        Err(e) => admin_error(e),
    }
}

async fn enable_plugin(State(st): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    set_enabled_route(st, id, headers, true)
}

async fn disable_plugin(State(st): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    set_enabled_route(st, id, headers, false)
}

fn set_enabled_route(st: AppState, id: String, headers: HeaderMap, enabled: bool) -> Response {
    if let Err(r) = require_ui(&st, &headers) {
        return *r;
    }
    match st.plugins.set_enabled(&id, enabled) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => admin_error(e),
    }
}

/// Removes the registry entry and tears the plugin down. The CLI deletes the
/// files afterwards.
async fn remove_plugin(State(st): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_ui(&st, &headers) {
        return *r;
    }
    match super::admin::remove_entry(st.plugins.paths(), &id) {
        Ok(_) => reloaded(&st),
        Err(e) => admin_error(e),
    }
}

/// `{action, key}`: `key` null clears the binding.
async fn bind_plugin(State(st): State<AppState>, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = require_ui(&st, &headers) {
        return *r;
    }
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let Some(action) = body.get("action").and_then(Value::as_str).filter(|a| a.starts_with(&format!("{id}:"))) else {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, format!("action must be a full id starting with {id}:"));
    };
    let key = body.get("key").and_then(Value::as_str).filter(|k| !k.is_empty());
    if !body.get("key").is_some_and(Value::is_null) && key.is_none() {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "key must be a non-empty string, or null to clear");
    }
    match super::admin::set_binding(st.plugins.paths(), &id, action, key) {
        Ok(()) => reloaded(&st),
        Err(e) => admin_error(e),
    }
}

async fn features() -> Json<Value> {
    Json(garage_plugin::features::features_response(env!("CARGO_PKG_VERSION")))
}

async fn reload(State(st): State<AppState>, headers: HeaderMap) -> Response {
    // Reload resets event-hook auto-disable, so a plugin must not be able to
    // call it for itself.
    if let Err(r) = require_ui(&st, &headers) {
        return *r;
    }
    match st.plugins.reload() {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => error_response(StatusCode::CONFLICT, e),
    }
}

/// The TUI reports the geometry of the slots plugins draw into (strip width
/// and filler range, max item columns, rail width), so run contexts can
/// carry it.
async fn put_ui_slots(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = require_ui(&st, &headers) {
        return *r;
    }
    if let Err(wait) = st.plugins.acquire_slots(Instant::now()) {
        let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
        let mut res = error_response(StatusCode::TOO_MANY_REQUESTS, "slot geometry updates are limited to 10 per second (burst 20)");
        res.headers_mut().insert(RETRY_AFTER, secs.to_string().parse().expect("digits"));
        return res;
    }
    if body.len() > limits::SLOTS_BODY_BYTES {
        return error_response(StatusCode::PAYLOAD_TOO_LARGE, format!("body is {} bytes; limit is {}", body.len(), limits::SLOTS_BODY_BYTES));
    }
    let v = match parse_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    if let Err(e) = validate_slot_geometry(&v) {
        return error_response(StatusCode::from_u16(e.status).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY), e.message);
    }
    st.plugins.set_ui_slots(v);
    Json(json!({"ok": true})).into_response()
}

async fn put_widget(
    State(st): State<AppState>,
    Path((id, widget)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let plugin = match authed_for(&st, &headers, &id, Permission::UiWidgets) {
        Ok(p) => p,
        Err(r) => return *r,
    };
    let manifest = plugin.manifest().expect("active plugin has a manifest").clone();
    let Some(decl) = manifest.widget(&widget) else {
        return error_response(StatusCode::NOT_FOUND, format!("plugin {id} declares no widget {widget:?}"));
    };
    if let Err(wait) = st.plugins.widgets().acquire(&id, Instant::now()) {
        let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
        let mut res = error_response(StatusCode::TOO_MANY_REQUESTS, "widget pushes are limited to 10 per second (burst 20)");
        res.headers_mut().insert(RETRY_AFTER, secs.to_string().parse().expect("digits"));
        return res;
    }
    if let Err(e) = check_body_len(body.len()) {
        return error_response(StatusCode::PAYLOAD_TOO_LARGE, e.message);
    }
    let put: WidgetPut = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, format!("invalid widget body: {e}")),
    };
    let slot = decl.slot.to_protocol();
    let put = match validate_put(slot, put) {
        Ok(p) => p,
        Err(e) => {
            let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY);
            return error_response(status, e.message);
        }
    };
    for click in click_actions(&put) {
        let a = click.action.as_str();
        if click.is_core() {
            if !manifest.has_permission(Permission::CoreActions) {
                return error_response(
                    StatusCode::FORBIDDEN,
                    format!("plugin {id} lacks permission core.actions (click.action {a:?} is a core action)"),
                );
            }
            match garage_plugin::core_actions::lookup(a) {
                Some(c) if c.plugin_invocable => {}
                Some(_) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, format!("click.action {a:?} cannot be triggered by plugins")),
                None => return error_response(StatusCode::UNPROCESSABLE_ENTITY, format!("click.action {a:?} is not a core action")),
            }
        } else {
            let (owner, local) = a.split_once(':').unwrap_or((id.as_str(), a));
            if owner == id && manifest.action(local).is_none() {
                return error_response(StatusCode::UNPROCESSABLE_ENTITY, format!("click.action {a:?}: plugin {id} has no action {local:?}"));
            }
        }
    }
    let outcome = st.plugins.widgets().put(&id, &widget, slot, put);
    let rev = match outcome {
        crate::plugins::widgets::PutOutcome::Changed { rev } | crate::plugins::widgets::PutOutcome::Unchanged { rev } => rev,
    };
    Json(json!({"ok": true, "rev": rev})).into_response()
}

async fn delete_widget(State(st): State<AppState>, Path((id, widget)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let plugin = match authed_for(&st, &headers, &id, Permission::UiWidgets) {
        Ok(p) => p,
        Err(r) => return *r,
    };
    if plugin.manifest().is_some_and(|m| m.widget(&widget).is_none()) {
        return error_response(StatusCode::NOT_FOUND, format!("plugin {id} declares no widget {widget:?}"));
    }
    let cleared = st.plugins.widgets().clear_widget(&id, &widget);
    Json(json!({"ok": true, "cleared": cleared, "rev": st.plugins.widgets().rev(&id)})).into_response()
}

/// Called by the TUI with the UI token, or by a plugin with its own token for
/// its own actions only.
async fn run_action(State(st): State<AppState>, Path((id, action)): Path<(String, String)>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(token) = bearer(&headers) else {
        return error_response(StatusCode::UNAUTHORIZED, "missing token (the UI token, or this plugin's own token)");
    };
    if !st.plugins.is_ui_token(token) {
        match st.plugins.authenticate(token) {
            None => return error_response(StatusCode::UNAUTHORIZED, "invalid token"),
            Some(p) if p.entry.id != id => {
                return error_response(
                    StatusCode::FORBIDDEN,
                    format!("this token belongs to plugin {}; running {id}'s actions needs the UI token", p.entry.id),
                );
            }
            Some(p) if !p.active() => return error_response(StatusCode::FORBIDDEN, format!("plugin {id} is disabled")),
            Some(_) => {}
        }
    }
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    match st.plugins.run_action(&id, &action, &body).await {
        Ok(run_id) => (StatusCode::ACCEPTED, Json(json!({"run_id": run_id.to_string()}))).into_response(),
        Err(RunError::UnknownPlugin) => error_response(StatusCode::NOT_FOUND, format!("no plugin {id}")),
        Err(RunError::UnknownAction) => error_response(StatusCode::NOT_FOUND, format!("plugin {id} has no action {action:?}")),
        Err(RunError::Disabled) => error_response(StatusCode::CONFLICT, format!("plugin {id} is disabled")),
        Err(RunError::PaneAction) => error_response(StatusCode::NOT_IMPLEMENTED, format!("action {action:?} opens a pane; panes are not available yet")),
    }
}

/// `POST /api/sessions/input`: type text into a session. Token REQUIRED,
/// permission `sessions.input`. The id is in the body because session ids
/// contain slashes.
async fn session_input(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let plugin = match authed(&st, &headers, Permission::SessionsInput) {
        Ok(p) => p,
        Err(r) => return *r,
    };
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let id = body.get("id").and_then(Value::as_str).unwrap_or("");
    let Some(text) = body.get("text").and_then(Value::as_str) else {
        return error_response(StatusCode::BAD_REQUEST, "text must be a string");
    };
    if !id.starts_with(GARAGE_PREFIX) {
        return error_response(StatusCode::FORBIDDEN, "refusing to touch non-garage sessions");
    }
    if !st.backend.has(id).await.unwrap_or(false) {
        return error_response(StatusCode::NOT_FOUND, format!("no such session: {id}"));
    }
    match st.backend.send_text(id, text).await {
        Ok(()) => {
            let log = st.plugins.0.log_for(&plugin.entry.id);
            log.append("host", &format!("sessions.input to {id} ({} bytes)", text.len())).await;
            Json(json!({"ok": true})).into_response()
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `GET /api/workspaces/{name}/closed`. Core data; a token is optional, but
/// one that is presented must be valid and hold `sessions.read`.
async fn closed(State(st): State<AppState>, Path(name): Path<String>, headers: HeaderMap) -> Response {
    if headers.contains_key(AUTHORIZATION) {
        if let Err(r) = authed(&st, &headers, Permission::SessionsRead) {
            return *r;
        }
    }
    if st.registry.get_workspace(&name).await.is_none() {
        return error_response(StatusCode::NOT_FOUND, format!("unknown workspace: {name}"));
    }
    Json(st.registry.list_closed(&name).await).into_response()
}
