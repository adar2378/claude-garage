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
use garage_protocol::widgets::{
    WidgetPut, check_body_len, click_actions, limits, validate_put, validate_slot_geometry,
};
use serde_json::{Value, json};

use super::panes::PaneError;
use super::{PluginState, RunError};
use crate::api::hooks::parse_body;
use crate::backend::tmux::GARAGE_PREFIX;
use crate::error::error_response;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/plugin/features", get(features))
        .route("/api/plugins", get(list))
        .route("/api/plugins/widgets", get(widgets_snapshot))
        .route("/api/plugins/ui/action", post(ui_action))
        .route("/api/plugins/{id}/notice", post(notice))
        .route("/api/plugins/panes", get(panes))
        .route("/api/plugins/{id}/panes/{pane}/open", post(open_pane))
        .route(
            "/api/plugins/{id}/panes/{pane}",
            axum::routing::delete(close_pane),
        )
        .route("/api/plugins/reload", post(reload))
        .route("/api/plugins/ui/slots", put(put_ui_slots))
        .route("/api/plugins/ui/challenge", post(ui_challenge))
        .route("/api/plugins/{id}", axum::routing::delete(remove_plugin))
        .route("/api/plugins/{id}/registration", put(register_plugin))
        .route("/api/plugins/{id}/enable", post(enable_plugin))
        .route("/api/plugins/{id}/disable", post(disable_plugin))
        .route("/api/plugins/{id}/bind", post(bind_plugin))
        .route(
            "/api/plugins/{id}/widgets/{widget}",
            put(put_widget).delete(delete_widget),
        )
        .route("/api/plugins/{id}/actions/{action}/run", post(run_action))
        .route("/api/sessions/input", post(session_input))
        .route("/api/workspaces/{name}/closed", get(closed))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    // An empty bearer is no credential at all.
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// A request's plugin, checked for the permission. 401 for a missing or
/// unknown token, 403 for a disabled plugin or a missing permission.
fn authed(
    st: &AppState,
    headers: &HeaderMap,
    perm: Permission,
) -> Result<PluginState, Box<Response>> {
    let Some(token) = bearer(headers) else {
        return Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            "missing plugin token (Authorization: Bearer $GARAGE_PLUGIN_TOKEN)",
        )));
    };
    let Some(plugin) = st.plugins.authenticate(token) else {
        return Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            "invalid plugin token",
        )));
    };
    let id = &plugin.entry.id;
    if !plugin.active() {
        return Err(Box::new(error_response(
            StatusCode::FORBIDDEN,
            format!("plugin {id} is disabled"),
        )));
    }
    if !plugin.manifest().is_some_and(|m| m.has_permission(perm)) {
        return Err(Box::new(error_response(
            StatusCode::FORBIDDEN,
            format!("plugin {id} lacks permission {}", perm.as_str()),
        )));
    }
    Ok(plugin)
}

/// Like [`authed`], and the token must belong to the plugin in the path.
fn authed_for(
    st: &AppState,
    headers: &HeaderMap,
    id: &str,
    perm: Permission,
) -> Result<PluginState, Box<Response>> {
    let plugin = authed(st, headers, perm)?;
    if plugin.entry.id != id {
        return Err(Box::new(error_response(
            StatusCode::FORBIDDEN,
            format!("this token belongs to plugin {}, not {id}", plugin.entry.id),
        )));
    }
    Ok(plugin)
}

/// The caller must present the UI token.
fn require_ui(st: &AppState, headers: &HeaderMap) -> Result<(), Box<Response>> {
    match bearer(headers) {
        None => Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            "missing UI token (Authorization: Bearer <contents of GARAGE_DIR/ui-token>)",
        ))),
        Some(t) if st.plugins.is_ui_token(t) => Ok(()),
        Some(_) => Err(Box::new(error_response(
            StatusCode::FORBIDDEN,
            "this route needs the UI token; a plugin token is not enough",
        ))),
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
    let Some(nonce) = body
        .get("nonce")
        .and_then(Value::as_str)
        .filter(|n| garage_protocol::auth::nonce_ok(n))
    else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "nonce must be 16 to 128 lowercase hex digits",
        );
    };
    match st.plugins.ui_proof(st.config.port, nonce) {
        Some(proof) => Json(json!({"proof": proof})).into_response(),
        None => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "this daemon has no UI token",
        ),
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

async fn register_plugin(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(r) = require_ui(&st, &headers) {
        return *r;
    }
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let source = match body
        .get("source")
        .cloned()
        .map(serde_json::from_value::<super::layout::Source>)
    {
        Some(Ok(s)) => s,
        _ => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "source must be {kind: github|link, ...}",
            );
        }
    };
    let Some(granted) = body
        .get("grantedPermissions")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        })
    else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "grantedPermissions must be an array of strings",
        );
    };
    if let Some(bad) = granted
        .iter()
        .find(|p| !garage_plugin::features::PERMISSIONS.contains(&p.as_str()))
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("unknown permission {bad:?}"),
        );
    }
    let reg = super::admin::Registration {
        id,
        source,
        granted_permissions: granted,
        manifest_sha256: body
            .get("manifestSha256")
            .and_then(Value::as_str)
            .map(str::to_owned),
        installed_at: body.get("installedAt").and_then(Value::as_i64),
    };
    match super::admin::register(st.plugins.paths(), &reg) {
        Ok(()) => reloaded(&st),
        Err(e) => admin_error(e),
    }
}

async fn enable_plugin(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    set_enabled_route(st, id, headers, true)
}

async fn disable_plugin(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
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
async fn remove_plugin(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_ui(&st, &headers) {
        return *r;
    }
    match super::admin::remove_entry(st.plugins.paths(), &id) {
        Ok(_) => reloaded(&st),
        Err(e) => admin_error(e),
    }
}

/// `{action, key}`: `key` null clears the binding.
async fn bind_plugin(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(r) = require_ui(&st, &headers) {
        return *r;
    }
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let Some(action) = body
        .get("action")
        .and_then(Value::as_str)
        .filter(|a| a.starts_with(&format!("{id}:")))
    else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("action must be a full id starting with {id}:"),
        );
    };
    let key = body
        .get("key")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty());
    if !body.get("key").is_some_and(Value::is_null) && key.is_none() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "key must be a non-empty string, or null to clear",
        );
    }
    match super::admin::set_binding(st.plugins.paths(), &id, action, key) {
        Ok(()) => reloaded(&st),
        Err(e) => admin_error(e),
    }
}

async fn features() -> Json<Value> {
    Json(garage_plugin::features::features_response(env!(
        "CARGO_PKG_VERSION"
    )))
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
        let mut res = error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "slot geometry updates are limited to 10 per second (burst 20)",
        );
        res.headers_mut()
            .insert(RETRY_AFTER, secs.to_string().parse().expect("digits"));
        return res;
    }
    if body.len() > limits::SLOTS_BODY_BYTES {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "body is {} bytes; limit is {}",
                body.len(),
                limits::SLOTS_BODY_BYTES
            ),
        );
    }
    let v = match parse_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    if let Err(e) = validate_slot_geometry(&v) {
        return error_response(
            StatusCode::from_u16(e.status).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            e.message,
        );
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
    let manifest = plugin
        .manifest()
        .expect("active plugin has a manifest")
        .clone();
    let Some(decl) = manifest.widget(&widget) else {
        return error_response(
            StatusCode::NOT_FOUND,
            format!("plugin {id} declares no widget {widget:?}"),
        );
    };
    if let Err(wait) = st.plugins.widgets().acquire(&id, Instant::now()) {
        let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
        let mut res = error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "widget pushes are limited to 10 per second (burst 20)",
        );
        res.headers_mut()
            .insert(RETRY_AFTER, secs.to_string().parse().expect("digits"));
        return res;
    }
    if let Err(e) = check_body_len(body.len()) {
        return error_response(StatusCode::PAYLOAD_TOO_LARGE, e.message);
    }
    let put: WidgetPut = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("invalid widget body: {e}"),
            );
        }
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
        if let Err(r) = check_action(&manifest, &id, &click.action) {
            return *r;
        }
    }
    let outcome = st.plugins.widgets().put(&id, &widget, slot, put);
    let rev = match outcome {
        crate::plugins::widgets::PutOutcome::Changed { rev }
        | crate::plugins::widgets::PutOutcome::Unchanged { rev } => rev,
    };
    Json(json!({"ok": true, "rev": rev})).into_response()
}

/// Whether `plugin` (with `manifest`) may name `action` as a click or UI
/// action: a core id needs `core.actions` and must be plugin-invocable; a
/// plugin id must be one this plugin's manifest declares (an id of another
/// plugin is not checked here, the TUI refuses what no manifest declared).
fn check_action(
    manifest: &garage_plugin::manifest::Manifest,
    id: &str,
    a: &str,
) -> Result<(), Box<Response>> {
    let refuse = |status, msg: String| Err(Box::new(error_response(status, msg)));
    if garage_protocol::widgets::is_core_action_id(a) {
        // Navigation (focus, view) is covered by the narrow ui.navigate;
        // everything else a plugin may trigger needs core.actions.
        let navigation = garage_plugin::core_actions::is_navigation(a);
        if !manifest.has_permission(Permission::CoreActions)
            && !(navigation && manifest.has_permission(Permission::UiNavigate))
        {
            let need = if navigation {
                "ui.navigate (or core.actions)"
            } else {
                "core.actions"
            };
            return refuse(
                StatusCode::FORBIDDEN,
                format!(
                    "plugin {id} lacks permission {need} (click.action {a:?} is a core action)"
                ),
            );
        }
        return match garage_plugin::core_actions::lookup(a) {
            Some(c) if c.plugin_invocable => Ok(()),
            Some(_) => refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("click.action {a:?} cannot be triggered by plugins"),
            ),
            None => refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("click.action {a:?} is not a core action"),
            ),
        };
    }
    let (owner, local) = a.split_once(':').unwrap_or((id, a));
    if owner != id {
        return refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "click.action {a:?}: plugin {id} may only target its own actions or a core id, not plugin {owner}'s"
            ),
        );
    }
    if manifest.action(local).is_none() {
        return refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("click.action {a:?}: plugin {id} has no action {local:?}"),
        );
    }
    Ok(())
}

/// The target of a `ui/action` must name things that exist: a `garage/<ws>/<label>`
/// session that is live or has a closed record (never a plugin pane or any other
/// backend id), a registered workspace, and the session inside that workspace
/// when both are given. Anything else is 422 and nothing reaches the TUI.
async fn check_target(
    st: &AppState,
    target: &garage_protocol::plugins::UiTarget,
) -> Result<(), Box<Response>> {
    let refuse = |msg: String| {
        Err(Box::new(error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            msg,
        )))
    };
    if let Some(ws) = &target.workspace
        && st.registry.get_workspace(ws).await.is_none()
    {
        return refuse(format!(
            "target.workspace: {ws:?} is not a registered workspace"
        ));
    }
    if let Some(session) = &target.session {
        let parts: Vec<&str> = session.split('/').collect();
        let shaped =
            parts.len() == 3 && parts[0] == "garage" && parts[1..].iter().all(|p| !p.is_empty());
        if !shaped {
            return refuse(format!(
                "target.session: {session:?} is not a garage/<workspace>/<label> session id"
            ));
        }
        let exists = st.backend.has(session).await.unwrap_or(false)
            || st.registry.get_closed(session).await.is_some()
            || st.registry.get_session_meta(session).await.is_some();
        if !exists {
            return refuse(format!("target.session: no session {session:?}"));
        }
        if let Some(ws) = &target.workspace
            && parts[1] != ws
        {
            return refuse(format!(
                "target.session {session:?} does not belong to workspace {ws:?}"
            ));
        }
    }
    Ok(())
}

async fn list(State(st): State<AppState>) -> Json<garage_protocol::plugins::PluginList> {
    Json(st.plugins.list())
}

#[derive(serde::Deserialize)]
struct WidgetsQuery {
    plugin: Option<String>,
}

async fn widgets_snapshot(
    State(st): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<WidgetsQuery>,
) -> Json<garage_protocol::plugins::WidgetsSnapshot> {
    Json(st.plugins.widget_snapshot(q.plugin.as_deref()))
}

/// Core ids a plugin's `ui/action` request may never name, though a widget
/// click (a human) may: they switch the engaged, typing target.
const UI_ACTION_REFUSED: &[&str] = &["session.engage"];

/// `POST /api/plugins/ui/action`: a plugin asks the TUI to run an action. The
/// daemon checks the token, `ui.invoke` and the action allowlist, then
/// forwards the request as an SSE `ui_action` frame; the TUI treats it as a
/// plugin trigger (it can arm a dangerous action, never confirm one). The body
/// is read field by field: nothing like `confirmed` is ever taken from it.
async fn ui_action(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let plugin = match authed(&st, &headers, Permission::UiInvoke) {
        Ok(p) => p,
        Err(r) => return *r,
    };
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let id = plugin.entry.id.clone();
    let manifest = plugin
        .manifest()
        .expect("active plugin has a manifest")
        .clone();
    let Some(action) = body.get("action").and_then(Value::as_str).filter(|a| {
        !a.is_empty() && a.len() <= limits::ACTION_LEN && !a.chars().any(char::is_control)
    }) else {
        return error_response(StatusCode::BAD_REQUEST, "action must be a non-empty string");
    };
    let args = body.get("args").filter(|a| !a.is_null()).cloned();
    if args
        .as_ref()
        .is_some_and(|a| serde_json::to_vec(a).map_or(true, |v| v.len() > limits::CLICK_ARGS_BYTES))
    {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("args: limit is {} bytes", limits::CLICK_ARGS_BYTES),
        );
    }
    let text = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
            .map(str::to_owned)
    };
    let target = garage_protocol::plugins::UiTarget {
        session: text(body.get("target").and_then(|t| t.get("session"))),
        workspace: text(body.get("target").and_then(|t| t.get("workspace"))),
    };
    if let Err(r) = check_action(&manifest, &id, action) {
        return *r;
    }
    // Where the user types is the user's to change: a plugin may not start
    // typing into a session on their behalf (a widget click, a human act, may).
    if UI_ACTION_REFUSED.contains(&action) {
        return error_response(
            StatusCode::FORBIDDEN,
            format!(
                "action {action:?} cannot be requested by a plugin (it moves where the user types)"
            ),
        );
    }
    if let Err(r) = check_target(&st, &target).await {
        return *r;
    }
    if let Err(wait) = st.plugins.acquire_ui_action(&id, Instant::now()) {
        let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
        let mut res = error_response(
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "ui actions are limited to {} per second per plugin",
                super::UI_ACTIONS_PER_SEC
            ),
        );
        res.headers_mut()
            .insert(RETRY_AFTER, secs.to_string().parse().expect("digits"));
        return res;
    }
    // A bare local id is this plugin's own action; frame it in full.
    let action = if garage_protocol::widgets::is_core_action_id(action) || action.contains(':') {
        action.to_owned()
    } else {
        format!("{id}:{action}")
    };
    st.bus.publish(crate::bus::DaemonEvent::UiAction {
        plugin: id,
        action,
        args,
        target,
    });
    (StatusCode::ACCEPTED, Json(json!({"ok": true}))).into_response()
}

/// `POST /api/plugins/{id}/notice {text, ttl_ms?}`: a short message for the
/// user, shown as a toast. The text is cleaned (no control or invisible
/// characters), trimmed and cut to [`limits::NOTICE_LEN`] characters; the
/// time is clamped; a plugin gets one notice a second (bursts of three).
async fn notice(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(r) = authed_for(&st, &headers, &id, Permission::UiNotice) {
        return *r;
    }
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let text: String =
        garage_protocol::widgets::sanitise(body.get("text").and_then(Value::as_str).unwrap_or(""))
            .trim()
            .chars()
            .take(limits::NOTICE_LEN)
            .collect();
    if text.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "text must be a non-empty string");
    }
    let ttl_ms = body.get("ttl_ms").and_then(Value::as_u64).map(|t| {
        t.clamp(
            u64::from(limits::NOTICE_TTL_MS_MIN),
            u64::from(limits::NOTICE_TTL_MS_MAX),
        ) as u32
    });
    if let Err(wait) = st.plugins.acquire_notice(&id, Instant::now()) {
        let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
        let mut res = error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "notices are limited to 1 per second (burst 3)",
        );
        res.headers_mut()
            .insert(RETRY_AFTER, secs.to_string().parse().expect("digits"));
        return res;
    }
    st.bus.publish(crate::bus::DaemonEvent::PluginNotice {
        plugin: id,
        message: text,
        ttl_ms: Some(ttl_ms.unwrap_or(limits::NOTICE_TTL_MS_DEFAULT)),
    });
    (StatusCode::ACCEPTED, Json(json!({"ok": true}))).into_response()
}

async fn panes(State(st): State<AppState>) -> Json<garage_protocol::plugins::PanesResponse> {
    Json(garage_protocol::plugins::PanesResponse {
        panes: st.plugins.live_panes().await,
    })
}

/// The caller of a pane route: the UI token (the TUI), or the plugin's own
/// token with `ui.panes`.
fn authed_pane_caller(st: &AppState, headers: &HeaderMap, id: &str) -> Result<(), Box<Response>> {
    match bearer(headers) {
        Some(t) if st.plugins.is_ui_token(t) => Ok(()),
        Some(_) => authed_for(st, headers, id, Permission::UiPanes).map(|_| ()),
        None => Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            "missing token (the UI token, or this plugin's own token)",
        ))),
    }
}

fn pane_error(id: &str, pane: &str, e: PaneError) -> Response {
    let status = match e {
        PaneError::UnknownPlugin | PaneError::UnknownPane => StatusCode::NOT_FOUND,
        PaneError::Disabled => StatusCode::CONFLICT,
        PaneError::TooMany => StatusCode::TOO_MANY_REQUESTS,
        PaneError::NoBackend | PaneError::Backend(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error_response(status, format!("pane {id}:{pane}: {e}"))
}

async fn open_pane(
    State(st): State<AppState>,
    Path((id, pane)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(r) = authed_pane_caller(&st, &headers, &id) {
        return *r;
    }
    let body = if body.is_empty() {
        json!({})
    } else {
        match parse_body(&body) {
            Ok(b) => b,
            Err(r) => return *r,
        }
    };
    match st.plugins.open_pane(&id, &pane, &body).await {
        Ok((rec, reused)) => {
            Json(json!({"ok": true, "session": rec.session, "reused": reused})).into_response()
        }
        Err(e) => pane_error(&id, &pane, e),
    }
}

async fn close_pane(
    State(st): State<AppState>,
    Path((id, pane)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = authed_pane_caller(&st, &headers, &id) {
        return *r;
    }
    match st.plugins.close_pane(&id, &pane).await {
        Ok(closed) => Json(json!({"ok": true, "closed": closed})).into_response(),
        Err(e) => pane_error(&id, &pane, e),
    }
}

async fn delete_widget(
    State(st): State<AppState>,
    Path((id, widget)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let plugin = match authed_for(&st, &headers, &id, Permission::UiWidgets) {
        Ok(p) => p,
        Err(r) => return *r,
    };
    if plugin
        .manifest()
        .is_some_and(|m| m.widget(&widget).is_none())
    {
        return error_response(
            StatusCode::NOT_FOUND,
            format!("plugin {id} declares no widget {widget:?}"),
        );
    }
    let cleared = st.plugins.widgets().clear_widget(&id, &widget);
    Json(json!({"ok": true, "cleared": cleared, "rev": st.plugins.widgets().rev(&id)}))
        .into_response()
}

/// Called by the TUI with the UI token, or by a plugin with its own token for
/// its own actions only.
async fn run_action(
    State(st): State<AppState>,
    Path((id, action)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(token) = bearer(&headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "missing token (the UI token, or this plugin's own token)",
        );
    };
    if !st.plugins.is_ui_token(token) {
        match st.plugins.authenticate(token) {
            None => return error_response(StatusCode::UNAUTHORIZED, "invalid token"),
            Some(p) if p.entry.id != id => {
                return error_response(
                    StatusCode::FORBIDDEN,
                    format!(
                        "this token belongs to plugin {}; running {id}'s actions needs the UI token",
                        p.entry.id
                    ),
                );
            }
            Some(p) if !p.active() => {
                return error_response(StatusCode::FORBIDDEN, format!("plugin {id} is disabled"));
            }
            Some(_) => {}
        }
    }
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    // An action that opens a pane is the same trigger with a different effect.
    if let Some(pane) = st.plugins.active(&id).and_then(|p| {
        p.manifest()
            .and_then(|m| m.action(&action).and_then(|a| a.pane.clone()))
    }) {
        // The UI token may open any pane; a plugin's own token needs ui.panes.
        if let Err(r) = authed_pane_caller(&st, &headers, &id) {
            return *r;
        }
        return match st.plugins.open_pane(&id, &pane, &body).await {
            Ok((rec, _)) => {
                (StatusCode::ACCEPTED, Json(json!({"pane": rec.session}))).into_response()
            }
            Err(e) => pane_error(&id, &pane, e),
        };
    }
    match st.plugins.run_action(&id, &action, &body).await {
        Ok(run_id) => (
            StatusCode::ACCEPTED,
            Json(json!({"run_id": run_id.to_string()})),
        )
            .into_response(),
        Err(RunError::UnknownPlugin) => {
            error_response(StatusCode::NOT_FOUND, format!("no plugin {id}"))
        }
        Err(RunError::UnknownAction) => error_response(
            StatusCode::NOT_FOUND,
            format!("plugin {id} has no action {action:?}"),
        ),
        Err(RunError::Disabled) => {
            error_response(StatusCode::CONFLICT, format!("plugin {id} is disabled"))
        }
        Err(RunError::PaneAction) => error_response(
            StatusCode::NOT_IMPLEMENTED,
            format!("action {action:?} opens a pane; panes are not available yet"),
        ),
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
        return error_response(
            StatusCode::FORBIDDEN,
            "refusing to touch non-garage sessions",
        );
    }
    if !st.backend.has(id).await.unwrap_or(false) {
        return error_response(StatusCode::NOT_FOUND, format!("no such session: {id}"));
    }
    match st.backend.send_text(id, text).await {
        Ok(()) => {
            let log = st.plugins.0.log_for(&plugin.entry.id);
            log.append(
                "host",
                &format!("sessions.input to {id} ({} bytes)", text.len()),
            )
            .await;
            Json(json!({"ok": true})).into_response()
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `GET /api/workspaces/{name}/closed`. Core data; a token is optional, but
/// one that is presented must be valid and hold `sessions.read`.
async fn closed(
    State(st): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    if headers.contains_key(AUTHORIZATION)
        && let Err(r) = authed(&st, &headers, Permission::SessionsRead)
    {
        return *r;
    }
    if st.registry.get_workspace(&name).await.is_none() {
        return error_response(StatusCode::NOT_FOUND, format!("unknown workspace: {name}"));
    }
    Json(st.registry.list_closed(&name).await).into_response()
}
