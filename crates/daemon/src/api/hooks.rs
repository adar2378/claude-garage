//! Port of `daemon/src/hooks.js`: Claude Code hook intake, snippet and install routes.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use garage_protocol::SessionStatus;
use serde_json::{json, Map, Value};

use crate::error::error_response;
use crate::install::{read_settings_or_refuse, write_settings_atomic, InstallError};
use crate::security::token_matches;
use crate::state::AppState;

/// Claude Code's Notification hook fires for real blockers AND for a ~60s idle
/// reminder ("Claude is waiting for your input") after a turn ends. The
/// reminder is idle semantics, and mapping it to needs-input would raise a
/// sticky false alarm. Unknown messages stay needs-input (fail toward
/// attention, never away from it). Case-insensitive match, like `/i`.
pub fn is_idle_reminder(message: Option<&str>) -> bool {
    message.is_some_and(|m| m.to_lowercase().contains("waiting for your input"))
}

fn hook_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/api/hooks/claude")
}

pub async fn hook_snippet(state: &AppState) -> anyhow::Result<Value> {
    let token = state.telemetry.deps.hook_token().await?;
    let url = format!("{}?token={token}", hook_url(state.config.port));
    let group = json!([{ "hooks": [{ "type": "http", "url": url }] }]);
    Ok(json!({
        "hooks": { "Notification": group.clone(), "Stop": group },
        "allowedHttpHookUrls": [format!("http://127.0.0.1:{}/*", state.config.port)],
    }))
}

fn group_has_any_url(group: &Value, urls: &[&str]) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hs| hs.iter().any(|h| h.get("url").and_then(Value::as_str).is_some_and(|u| urls.contains(&u))))
}

/// Merges the snippet into `settings`: a hook group is appended only when no
/// existing group for that event already carries one of its URLs (the URL
/// embeds the stable per-install token), so a second application is a no-op.
pub fn merge_hook_snippet(settings: &Map<String, Value>, snippet: &Value) -> (Map<String, Value>, bool) {
    let mut merged = settings.clone();
    let mut changed = false;

    let mut hooks = merged.get("hooks").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(events) = snippet.get("hooks").and_then(Value::as_object) {
        for (event, groups) in events {
            let mut existing = hooks.get(event).and_then(Value::as_array).cloned().unwrap_or_default();
            for group in groups.as_array().into_iter().flatten() {
                let urls: Vec<&str> = group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|h| h.get("url").and_then(Value::as_str))
                    .collect();
                if !existing.iter().any(|g| group_has_any_url(g, &urls)) {
                    existing.push(group.clone());
                    changed = true;
                }
            }
            hooks.insert(event.clone(), Value::Array(existing));
        }
    }
    merged.insert("hooks".into(), Value::Object(hooks));

    // A JS Set: existing duplicates collapse too.
    let mut allowed: Vec<Value> = Vec::new();
    for v in merged.get("allowedHttpHookUrls").and_then(Value::as_array).into_iter().flatten() {
        if !allowed.contains(v) {
            allowed.push(v.clone());
        }
    }
    for url in snippet.get("allowedHttpHookUrls").and_then(Value::as_array).into_iter().flatten() {
        if !allowed.contains(url) {
            allowed.push(url.clone());
            changed = true;
        }
    }
    merged.insert("allowedHttpHookUrls".into(), Value::Array(allowed));

    (merged, changed)
}

/// The `token` query param, only if given exactly once (Fastify turns a
/// repeated param into an array, which never matches).
pub(crate) fn query_token(query: &[(String, String)]) -> Option<&str> {
    let mut it = query.iter().filter(|(k, _)| k == "token");
    let first = it.next()?;
    it.next().is_none().then_some(first.1.as_str())
}

/// `req.body ?? {}`: an empty body is `{}`; malformed JSON is a 400.
pub(crate) fn parse_body(body: &Bytes) -> Result<Value, Box<Response>> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Null) => Ok(json!({})),
        Ok(v) => Ok(v),
        Err(e) => Err(Box::new(error_response(StatusCode::BAD_REQUEST, format!("invalid JSON body: {e}")))),
    }
}

pub(crate) fn install_error(err: InstallError, fallback: &str) -> Response {
    let status = StatusCode::from_u16(err.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let message = if err.message.is_empty() { fallback.to_owned() } else { err.message };
    error_response(status, message)
}

pub(crate) fn internal(err: anyhow::Error) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

/// Auth: per-install token in the URL (Claude's HTTP hooks are URL-only
/// config). Combined with the Origin allowlist, a foreign browser page is
/// stopped by Origin and anything else without the token is stopped here.
pub async fn claude(
    State(state): State<AppState>,
    Query(query): Query<Vec<(String, String)>>,
    body: Bytes,
) -> Response {
    let expected = match state.telemetry.deps.hook_token().await {
        Ok(t) => t,
        Err(e) => return internal(e),
    };
    if !token_matches(query_token(&query), &expected) {
        return error_response(StatusCode::UNAUTHORIZED, "missing or invalid hook token");
    }
    let payload = match parse_body(&body) {
        Ok(p) => p,
        Err(r) => return *r,
    };

    let event = payload.get("hook_event_name").and_then(Value::as_str);
    let status = match event {
        Some("Notification") => SessionStatus::NeedsInput,
        Some("Stop") => SessionStatus::Done,
        _ => return Json(json!({"ok": true, "ignored": true})).into_response(),
    };
    let message = payload.get("message").and_then(Value::as_str);
    if event == Some("Notification") && is_idle_reminder(message) {
        // Idle reminder: leave the state alone (Stop already set done).
        return Json(json!({"ok": true, "ignored": "idle-reminder"})).into_response();
    }

    let session_id = payload.get("session_id").and_then(Value::as_str).filter(|s| !s.is_empty());
    let cwd = payload.get("cwd").and_then(Value::as_str).filter(|s| !s.is_empty());
    let ids = state.telemetry.deps.resolve_session_ids(session_id, cwd).await;
    // A Notification's text rides into the status store alongside needs-input;
    // other events pass no message (cleared on any transition away).
    let message = (status == SessionStatus::NeedsInput).then(|| message.map(str::to_owned));
    for id in &ids {
        state.telemetry.deps.set_status(id, status, message.clone());
    }
    Json(json!({"ok": true, "applied": ids})).into_response()
}

pub async fn snippet(State(state): State<AppState>) -> Response {
    match hook_snippet(&state).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

/// Called by the TUI's `I` key: merge the snippet into settings.json
/// server-side (parse-or-refuse, backup, atomic write).
pub async fn install(State(state): State<AppState>) -> Response {
    match install_hooks(&state).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => install_error(e, "hook install failed"),
    }
}

async fn install_hooks(state: &AppState) -> Result<Value, InstallError> {
    let path = state.telemetry.settings_path();
    let settings = read_settings_or_refuse(&path).await?;
    let snippet = hook_snippet(state).await.map_err(|e| InstallError { status: 500, message: e.to_string() })?;
    let (merged, changed) = merge_hook_snippet(&settings.settings, &snippet);
    if !changed {
        return Ok(json!({"ok": true, "installed": true, "alreadyInstalled": true, "backup": null}));
    }
    let backup = write_settings_atomic(&path, settings.raw.as_deref(), &merged).await?;
    Ok(json!({"ok": true, "installed": true, "alreadyInstalled": false, "backup": backup}))
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A fake `TelemetryDeps` and an `AppState` rooted in temp dirs, shared by
    //! the hooks / statusline / usage tests.
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::backend::tmux::TmuxBackend;
    use crate::os::UnsupportedOs;
    use crate::state::context::{Telemetry, TelemetryDeps};
    use crate::Config;

    pub const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

    pub type StatusCall = (String, SessionStatus, Option<Option<String>>);

    #[derive(Default)]
    pub struct FakeDeps {
        /// session ids a payload resolves to
        pub resolves: Mutex<Vec<String>>,
        pub statuses: Mutex<Vec<StatusCall>>,
        pub last_resolve: Mutex<Option<(Option<String>, Option<String>)>>,
    }

    #[async_trait]
    impl TelemetryDeps for FakeDeps {
        async fn hook_token(&self) -> anyhow::Result<String> {
            Ok(TOKEN.into())
        }
        fn set_status(&self, id: &str, state: SessionStatus, message: Option<Option<String>>) {
            self.statuses.lock().unwrap().push((id.into(), state, message));
        }
        async fn resolve_session_ids(&self, session_id: Option<&str>, cwd: Option<&str>) -> Vec<String> {
            *self.last_resolve.lock().unwrap() = Some((session_id.map(Into::into), cwd.map(Into::into)));
            self.resolves.lock().unwrap().clone()
        }
    }

    pub struct Harness {
        pub state: AppState,
        pub deps: Arc<FakeDeps>,
        pub claude_home: tempfile::TempDir,
    }

    impl Harness {
        pub fn new(port: u16) -> Self {
            let claude_home = tempfile::tempdir().unwrap();
            let deps = Arc::new(FakeDeps::default());
            let config = Config {
                port,
                garage_dir: "/nonexistent".into(),
                tmux_socket: None,
                ui_origins: vec![],
            };
            let mut state = AppState::new(config, Arc::new(TmuxBackend::new(None)), Arc::new(UnsupportedOs));
            state.telemetry = Telemetry::new(claude_home.path().to_path_buf(), deps.clone());
            Self { state, deps, claude_home }
        }

        pub async fn call(&self, method: &str, uri: &str, body: Option<&str>) -> (StatusCode, Value) {
            let mut req = Request::builder().method(method).uri(uri);
            if body.is_some() {
                req = req.header("content-type", "application/json");
            }
            let req = req.body(Body::from(body.unwrap_or("").to_owned())).unwrap();
            let res = crate::app(self.state.clone()).oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    const SNIPPET_URL: &str = "http://127.0.0.1:4747/api/hooks/claude?token=abc";

    fn snippet_fixture() -> Value {
        let group = json!([{"hooks": [{"type": "http", "url": SNIPPET_URL}]}]);
        json!({
            "hooks": {"Notification": group.clone(), "Stop": group},
            "allowedHttpHookUrls": ["http://127.0.0.1:4747/*"],
        })
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    // JS: hooks.test.js "merge preserves unrelated settings and hooks"
    #[test]
    fn merge_preserves_unrelated_settings_and_hooks() {
        let settings = obj(json!({
            "model": "opus",
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo mine"}]}]},
        }));
        let (merged, changed) = merge_hook_snippet(&settings, &snippet_fixture());
        assert!(changed);
        assert_eq!(merged["model"], "opus");
        let stop: Vec<&str> = merged["hooks"]["Stop"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["hooks"].as_array().unwrap())
            .map(|h| h.get("command").or(h.get("url")).unwrap().as_str().unwrap())
            .collect();
        assert!(stop.contains(&"echo mine"));
        assert_eq!(merged["hooks"]["Notification"].as_array().unwrap().len(), 1);
        assert_eq!(merged["allowedHttpHookUrls"], json!(["http://127.0.0.1:4747/*"]));
    }

    // JS: "merge is idempotent — second application changes nothing"
    #[test]
    fn merge_is_idempotent() {
        let (first, _) = merge_hook_snippet(&Map::new(), &snippet_fixture());
        let (second, changed) = merge_hook_snippet(&first, &snippet_fixture());
        assert!(!changed);
        assert_eq!(second["hooks"]["Notification"].as_array().unwrap().len(), 1);
        assert_eq!(second["hooks"]["Stop"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn merge_keeps_existing_key_order() {
        let settings = obj(json!({"zeta": 1, "alpha": 2}));
        let (merged, _) = merge_hook_snippet(&settings, &snippet_fixture());
        assert_eq!(merged.keys().collect::<Vec<_>>(), ["zeta", "alpha", "hooks", "allowedHttpHookUrls"]);
    }

    // JS: "idle reminder is classified as noise; blockers and unknowns are not"
    #[test]
    fn idle_reminder_is_noise_blockers_and_unknowns_are_not() {
        assert!(is_idle_reminder(Some("Claude is waiting for your input")));
        assert!(is_idle_reminder(Some("CLAUDE IS WAITING FOR YOUR INPUT")));
        assert!(!is_idle_reminder(Some("Claude needs your permission to use Bash")));
        assert!(!is_idle_reminder(None));
        assert!(!is_idle_reminder(Some("")));
    }

    // --- routes (new in Rust: the JS suite only covered the pure parts) ---

    fn url(token: &str) -> String {
        format!("/api/hooks/claude?token={token}")
    }

    #[tokio::test]
    async fn hook_post_without_or_with_a_wrong_token_is_401() {
        let h = Harness::new(4747);
        for u in ["/api/hooks/claude", "/api/hooks/claude?token=wrong", "/api/hooks/claude?token=a&token=b"] {
            let (s, body) = h.call("POST", u, Some("{}")).await;
            assert_eq!((s, body), (StatusCode::UNAUTHORIZED, json!({"error": "missing or invalid hook token"})), "{u}");
        }
    }

    #[tokio::test]
    async fn hook_post_ignores_other_events() {
        let h = Harness::new(4747);
        let (s, body) = h.call("POST", &url(TOKEN), Some(r#"{"hook_event_name":"PreToolUse"}"#)).await;
        assert_eq!((s, body), (StatusCode::OK, json!({"ok": true, "ignored": true})));
        // no body at all is {} -> ignored
        let (s, body) = h.call("POST", &url(TOKEN), None).await;
        assert_eq!((s, body), (StatusCode::OK, json!({"ok": true, "ignored": true})));
        assert!(h.deps.statuses.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn hook_post_filters_the_idle_reminder() {
        let h = Harness::new(4747);
        *h.deps.resolves.lock().unwrap() = vec!["garage/a".into()];
        let (s, body) = h
            .call("POST", &url(TOKEN), Some(r#"{"hook_event_name":"Notification","message":"Claude is waiting for your input"}"#))
            .await;
        assert_eq!((s, body), (StatusCode::OK, json!({"ok": true, "ignored": "idle-reminder"})));
        assert!(h.deps.statuses.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn notification_sets_needs_input_with_its_message_and_stop_sets_done_without() {
        let h = Harness::new(4747);
        *h.deps.resolves.lock().unwrap() = vec!["garage/a".into(), "garage/b".into()];
        let (s, body) = h
            .call("POST", &url(TOKEN), Some(r#"{"hook_event_name":"Notification","message":"needs perm","session_id":"s1","cwd":"/w"}"#))
            .await;
        assert_eq!((s, body), (StatusCode::OK, json!({"ok": true, "applied": ["garage/a", "garage/b"]})));
        assert_eq!(*h.deps.last_resolve.lock().unwrap(), Some((Some("s1".into()), Some("/w".into()))));
        // Notification without a message passes null (Some(None)), not undefined.
        h.call("POST", &url(TOKEN), Some(r#"{"hook_event_name":"Notification"}"#)).await;
        h.call("POST", &url(TOKEN), Some(r#"{"hook_event_name":"Stop"}"#)).await;
        let got = h.deps.statuses.lock().unwrap().clone();
        assert_eq!(got[0], ("garage/a".into(), SessionStatus::NeedsInput, Some(Some("needs perm".into()))));
        assert_eq!(got[2], ("garage/a".into(), SessionStatus::NeedsInput, Some(None)));
        assert_eq!(got[4], ("garage/a".into(), SessionStatus::Done, None));
    }

    #[tokio::test]
    async fn malformed_json_body_is_400() {
        let h = Harness::new(4747);
        let (s, _) = h.call("POST", &url(TOKEN), Some("{nope")).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn snippet_embeds_the_token_and_port() {
        let h = Harness::new(47390);
        let (s, body) = h.call("GET", "/api/hooks/snippet", None).await;
        assert_eq!(s, StatusCode::OK);
        let u = format!("http://127.0.0.1:47390/api/hooks/claude?token={TOKEN}");
        assert_eq!(body["hooks"]["Notification"][0]["hooks"][0], json!({"type": "http", "url": u}));
        assert_eq!(body["hooks"]["Stop"][0]["hooks"][0]["url"], u);
        assert_eq!(body["allowedHttpHookUrls"], json!(["http://127.0.0.1:47390/*"]));
    }

    #[tokio::test]
    async fn install_creates_settings_then_is_idempotent_and_backs_up_existing() {
        let h = Harness::new(47390);
        let path = h.state.telemetry.settings_path();

        let (s, body) = h.call("POST", "/api/hooks/install", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body, json!({"ok": true, "installed": true, "alreadyInstalled": false, "backup": null}));
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["hooks"]["Stop"].as_array().unwrap().len(), 1);

        let before = std::fs::read_to_string(&path).unwrap();
        let (_, body) = h.call("POST", "/api/hooks/install", None).await;
        assert_eq!(body, json!({"ok": true, "installed": true, "alreadyInstalled": true, "backup": null}));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

        // An unrelated user setting + hook survive, and the pre-install file is backed up.
        std::fs::write(&path, r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo mine"}]}]}}"#).unwrap();
        let (_, body) = h.call("POST", "/api/hooks/install", None).await;
        assert_eq!(body["alreadyInstalled"], false);
        let backup = body["backup"].as_str().unwrap();
        assert!(std::fs::read_to_string(backup).unwrap().contains("echo mine"));
        let merged: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(merged["model"], "opus");
        assert_eq!(merged["hooks"]["Stop"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn install_refuses_a_corrupt_settings_file_with_422_leaving_it_untouched() {
        let h = Harness::new(47390);
        let path = h.state.telemetry.settings_path();
        std::fs::write(&path, "{ not: valid json !!").unwrap();
        let (s, body) = h.call("POST", "/api/hooks/install", None).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["error"].as_str().unwrap().contains("not valid JSON"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not: valid json !!");
    }
}
