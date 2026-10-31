//! Port of the routes in `daemon/src/statusline.js`: statusline intake,
//! snippet, install, and `GET /api/usage` (which `sessions.js` owned).

use std::path::Path;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::hooks::{install_error, internal, parse_body, query_token};
use crate::error::error_response;
use crate::install::{read_settings_or_refuse, write_settings_atomic, InstallError};
use crate::security::token_matches;
use crate::state::context::clamp_percentage;
use crate::state::AppState;

/// Single-quote a string for safe embedding in a POSIX sh script: close the
/// quote, emit an escaped literal quote, reopen it.
fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The chaining wrapper: a tiny POSIX sh script (not an inline settings.json
/// command string) so chaining an arbitrary pre-existing command never fights
/// JSON/shell quoting. `original_command` is the pre-existing statusLine
/// command to chain (its stdout rides through unchanged), if any.
pub fn wrapper_script(token: &str, port: u16, original_command: Option<&str>) -> String {
    let url = format!("http://127.0.0.1:{port}/api/statusline/claude?token={token}");
    let chain = original_command
        .map(|c| format!("printf '%s' \"$INPUT\" | sh -c {}\n", sh_single_quote(c)))
        .unwrap_or_default(); // no pre-existing statusline: post and print nothing
    format!(
        "#!/bin/sh\n\
         # claude-garage statusline wrapper — auto-generated, do not hand-edit.\n\
         # Reinstall (POST /api/statusline/install) regenerates this file.\n\
         INPUT=\"$(cat)\"\n\
         printf '%s' \"$INPUT\" | curl -fsS --max-time 1 -X POST {} \
         -H \"Content-Type: application/json\" --data-binary @- >/dev/null 2>&1 &\n\
         {chain}",
        sh_single_quote(&url)
    )
}

fn wrapper_command(wrapper_path: &Path) -> String {
    format!("sh {}", sh_single_quote(&wrapper_path.to_string_lossy()))
}

async fn write_wrapper_script(path: &Path, content: &str) -> Result<(), InstallError> {
    let io = |e: std::io::Error| InstallError { status: 500, message: e.to_string() };
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".garage-tmp-{}", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    tokio::fs::write(&tmp, content).await.map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).await.map_err(io)?;
    }
    tokio::fs::rename(&tmp, path).await.map_err(io)
}

pub async fn claude(
    State(state): State<AppState>,
    Query(query): Query<Vec<(String, String)>>,
    body: Bytes,
) -> Response {
    let t = &state.telemetry;
    let expected = match t.deps.hook_token().await {
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

    let session_id = payload.get("session_id").and_then(Value::as_str).filter(|s| !s.is_empty());
    let cwd = payload.get("cwd").and_then(Value::as_str).filter(|s| !s.is_empty());
    let ids = t.deps.resolve_session_ids(session_id, cwd).await;
    if ids.is_empty() {
        // Unresolvable session: nothing recorded (context or account-wide rate
        // limits); a post that cannot be attributed tells us nothing trustworthy.
        return Json(json!({"ok": true, "ignored": true})).into_response();
    }

    let window = payload.get("context_window");
    if let Some(pct) = window.and_then(|w| w.get("used_percentage")).and_then(clamp_percentage) {
        for id in &ids {
            t.context.set_context(id, pct);
        }
    }
    // Teach the transcript fallback this model's real window.
    t.transcripts.record_model_window(
        payload.get("model").and_then(|m| m.get("id")).and_then(Value::as_str),
        window.and_then(|w| w.get("context_window_size")).and_then(Value::as_f64),
    );
    if let Some(limits) = payload.get("rate_limits") {
        t.context.set_rate_limits(limits);
    }
    Json(json!({"ok": true, "applied": ids})).into_response()
}

pub async fn snippet(State(state): State<AppState>) -> Response {
    let t = &state.telemetry;
    match t.deps.hook_token().await {
        Ok(token) => Json(json!({
            "statusLine": {"type": "command", "command": wrapper_command(&t.wrapper_path())},
            "script": wrapper_script(&token, state.config.port, None),
        }))
        .into_response(),
        Err(e) => internal(e),
    }
}

/// Same safety order as the hooks install: parse-or-refuse, compute the merge,
/// backup + atomic write only if something actually changes. Idempotent when
/// `statusLine.command` already is our wrapper command.
pub async fn install(State(state): State<AppState>) -> Response {
    match install_statusline(&state).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => install_error(e, "statusline install failed"),
    }
}

async fn install_statusline(state: &AppState) -> Result<Value, InstallError> {
    let t = &state.telemetry;
    let _guard = t.install_lock.lock().await;
    let (settings_path, wrapper_path) = (t.settings_path(), t.wrapper_path());
    let settings = read_settings_or_refuse(&settings_path).await?;
    let token = t.deps.hook_token().await.map_err(|e| InstallError { status: 500, message: e.to_string() })?;
    let command = wrapper_command(&wrapper_path);

    let existing = settings.settings.get("statusLine").filter(|v| !v.is_null());
    if existing.and_then(|e| e.get("command")).and_then(Value::as_str) == Some(command.as_str()) {
        return Ok(json!({"ok": true, "installed": true, "alreadyInstalled": true, "chained": null, "backup": null}));
    }

    // Only a `type: "command"` statusLine has a shell command we can chain;
    // anything else is "nothing to chain".
    let original = existing
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("command"))
        .and_then(|e| e.get("command"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    write_wrapper_script(&wrapper_path, &wrapper_script(&token, state.config.port, original.as_deref())).await?;

    let mut merged = settings.settings.clone();
    merged.insert("statusLine".into(), json!({"type": "command", "command": command}));
    let backup = write_settings_atomic(&settings_path, settings.raw.as_deref(), &merged).await?;

    Ok(json!({
        "ok": true,
        "installed": true,
        "alreadyInstalled": false,
        "chained": original.is_some(),
        "backup": backup,
    }))
}

/// `GET /api/usage`: account-wide rate limits from the most recent statusline
/// posts, nulls until one arrives.
pub async fn usage(State(state): State<AppState>) -> Json<Value> {
    Json(state.telemetry.context.rate_limits().to_json())
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    use super::super::hooks::test_support::*;
    use super::*;

    fn url(token: &str) -> String {
        format!("/api/statusline/claude?token={token}")
    }

    // JS: statusline.test.js "401 on missing/invalid token"
    #[tokio::test]
    async fn post_with_a_missing_or_invalid_token_is_401() {
        let h = Harness::new(47391);
        for u in ["/api/statusline/claude", "/api/statusline/claude?token=totally-wrong"] {
            let (s, _) = h.call("POST", u, Some(r#"{"session_id":"whatever"}"#)).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{u}");
        }
    }

    // JS: "unresolvable session is {ignored:true}, no error"
    #[tokio::test]
    async fn unresolvable_session_is_ignored_and_records_nothing() {
        let h = Harness::new(47391);
        let body = r#"{"session_id":"no-such","cwd":"/no/such","context_window":{"used_percentage":99},
            "rate_limits":{"five_hour":{"used_percentage":5}}}"#;
        let (s, res) = h.call("POST", &url(TOKEN), Some(body)).await;
        assert_eq!((s, res), (StatusCode::OK, json!({"ok": true, "ignored": true})));
        assert_eq!(h.state.telemetry.context.rate_limits().five_hour, None);
    }

    // JS: "a resolvable session stores clamped context — locked scenario"
    // (the JS test used real tmux + cwd fallback; here resolution is the fake dep).
    #[tokio::test]
    async fn resolvable_session_stores_clamped_context() {
        let h = Harness::new(47391);
        *h.deps.resolves.lock().unwrap() = vec!["garage/x/one".into()];
        let (s, res) = h
            .call("POST", &url(TOKEN), Some(r#"{"cwd":"/w","context_window":{"used_percentage":142}}"#))
            .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(res, json!({"ok": true, "applied": ["garage/x/one"]}));
        assert_eq!(
            h.state.telemetry.context.statusline_context("garage/x/one"),
            Some(json!({"usedPercentage": 100, "source": "statusline"}))
        );
        assert_eq!(*h.deps.last_resolve.lock().unwrap(), Some((None, Some("/w".into()))));
    }

    // JS: "model.id + context_window_size teach the transcript fallback"
    #[tokio::test]
    async fn model_id_and_window_size_teach_the_transcript_fallback() {
        let h = Harness::new(47391);
        *h.deps.resolves.lock().unwrap() = vec!["garage/x/w".into()];
        let body = r#"{"cwd":"/w","model":{"id":"claude-window-test"},
            "context_window":{"used_percentage":12,"context_window_size":400000}}"#;
        let (s, _) = h.call("POST", &url(TOKEN), Some(body)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(h.state.telemetry.transcripts.window_for_model(Some("claude-window-test")), 400_000);
    }

    // JS: "rate_limits update the account-wide store even without context_window"
    #[tokio::test]
    async fn rate_limits_update_the_account_store_even_without_context_window() {
        let h = Harness::new(47391);
        *h.deps.resolves.lock().unwrap() = vec!["garage/x/r".into()];
        let body = r#"{"cwd":"/w","rate_limits":{"five_hour":{"used_percentage":33,"resets_at":"later"}}}"#;
        let (s, _) = h.call("POST", &url(TOKEN), Some(body)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(h.state.telemetry.context.statusline_context("garage/x/r"), None);
        assert_eq!(h.state.telemetry.context.rate_limits().five_hour.unwrap().used_percentage, 33.0);
    }

    // JS: "GET /api/statusline/snippet returns a wrapper definition without needing settings.json"
    #[tokio::test]
    async fn snippet_returns_a_wrapper_definition_without_settings_json() {
        let h = Harness::new(47391);
        let (s, body) = h.call("GET", "/api/statusline/snippet", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body["statusLine"]["type"], "command");
        assert!(body["statusLine"]["command"].as_str().unwrap().contains("garage-statusline.sh"));
        let script = body["script"].as_str().unwrap();
        assert!(script.contains("curl"));
        assert!(script.contains(&format!("api/statusline/claude?token={TOKEN}")));
        assert!(script.contains(":47391/"));
    }

    // --- usage.test.js, in one sequence (the JS suite relied on process isolation) ---

    #[tokio::test]
    async fn usage_is_null_then_reflects_and_merges_posts() {
        let h = Harness::new(47391);
        let (s, body) = h.call("GET", "/api/usage", None).await;
        assert_eq!((s, body), (StatusCode::OK, json!({"fiveHour": null, "sevenDay": null})));

        h.state.telemetry.context.set_rate_limits(
            &json!({"five_hour": {"used_percentage": 23.5, "resets_at": "2026-08-30T18:00:00Z"}}),
        );
        let (_, body) = h.call("GET", "/api/usage", None).await;
        assert_eq!(body["fiveHour"], json!({"usedPercentage": 23.5, "resetsAt": "2026-08-30T18:00:00Z"}));
        assert_eq!(body["sevenDay"], Value::Null);

        h.state
            .telemetry
            .context
            .set_rate_limits(&json!({"seven_day": {"used_percentage": 61, "resets_at": "next-week"}}));
        let (_, body) = h.call("GET", "/api/usage", None).await;
        assert_eq!(body["fiveHour"]["usedPercentage"], 23.5, "retained from the earlier post");
        assert_eq!(body["sevenDay"], json!({"usedPercentage": 61, "resetsAt": "next-week"}));
    }

    // --- statusline.install.test.js ---

    fn run_with_stdin(script: &Path, input: &str) -> String {
        use std::io::Write;
        let mut child = Command::new("sh")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
    }

    #[tokio::test]
    async fn install_flow_create_idempotent_corrupt_chain_rechain_and_fresh() {
        let h = Harness::new(47392); // never the real daemon's 4747
        let settings_path = h.state.telemetry.settings_path();
        let wrapper = h.state.telemetry.wrapper_path();

        // install with no pre-existing settings.json
        let (s, body) = h.call("POST", "/api/statusline/install", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body["installed"], true);
        assert_eq!(body["alreadyInstalled"], false);
        assert_eq!(body["chained"], false);
        assert_eq!(body["backup"], Value::Null);
        let settings: Value = serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(settings["statusLine"]["type"], "command");
        assert!(settings["statusLine"]["command"].as_str().unwrap().contains("garage-statusline.sh"));
        let script = std::fs::read_to_string(&wrapper).unwrap();
        assert!(script.contains("curl") && script.contains("api/statusline/claude?token="));
        assert!(crate::test_support::is_executable(&wrapper), "executable");

        // second install is idempotent: no backup, nothing rewritten
        let before = std::fs::read_to_string(&settings_path).unwrap();
        let (_, body) = h.call("POST", "/api/statusline/install", None).await;
        assert_eq!(body["alreadyInstalled"], true);
        assert_eq!(body["backup"], Value::Null);
        assert_eq!(body["chained"], Value::Null);
        assert_eq!(std::fs::read_to_string(&settings_path).unwrap(), before);

        // corrupt settings.json: 422, byte-for-byte untouched
        std::fs::write(&settings_path, "{ not: valid json !!").unwrap();
        let (s, _) = h.call("POST", "/api/statusline/install", None).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(std::fs::read_to_string(&settings_path).unwrap(), "{ not: valid json !!");

        // an existing non-garage statusLine is captured and chained; output byte-identical
        let original = h.claude_home.path().join("fake-original.sh");
        std::fs::write(&original, "#!/bin/sh\necho \"ORIGINAL-OUTPUT-MARKER\"\n").unwrap();
        crate::test_support::make_executable(&original);
        std::fs::write(
            &settings_path,
            json!({"model": "opus", "statusLine": {"type": "command", "command": original}}).to_string(),
        )
        .unwrap();
        let (s, body) = h.call("POST", "/api/statusline/install", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!((body["alreadyInstalled"].clone(), body["chained"].clone()), (json!(false), json!(true)));
        let backup = body["backup"].as_str().expect("a backup must be taken");
        assert!(std::fs::read_to_string(backup).unwrap().contains("fake-original.sh"));
        let settings: Value = serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(settings["model"], "opus");
        assert!(settings["statusLine"]["command"].as_str().unwrap().contains("garage-statusline.sh"));
        // The background curl targets a port nothing listens on and is silenced.
        let original_alone = run_with_stdin(&original, "");
        let wrapped = run_with_stdin(&wrapper, r#"{"session_id":"whatever","context_window":{"used_percentage":10}}"#);
        assert_eq!(wrapped, original_alone);
        assert_eq!(wrapped, "ORIGINAL-OUTPUT-MARKER\n");

        // reinstalling over garage's own wrapper is idempotent (no self-chaining)
        let before = std::fs::read_to_string(&settings_path).unwrap();
        let (_, body) = h.call("POST", "/api/statusline/install", None).await;
        assert_eq!(body["alreadyInstalled"], true);
        assert_eq!(std::fs::read_to_string(&settings_path).unwrap(), before);

        // a fresh install with no pre-existing statusLine prints nothing
        std::fs::write(&settings_path, "{}\n").unwrap();
        let (_, body) = h.call("POST", "/api/statusline/install", None).await;
        assert_eq!(body["chained"], false);
        assert_eq!(run_with_stdin(&wrapper, r#"{"session_id":"x"}"#), "");
    }

    #[test]
    fn sh_single_quote_escapes_embedded_quotes() {
        assert_eq!(sh_single_quote("a'b"), r"'a'\''b'");
        assert_eq!(sh_single_quote("plain"), "'plain'");
    }

    #[test]
    fn wrapper_script_has_the_exact_js_shape() {
        let s = wrapper_script("tok", 4747, Some("echo 'hi'"));
        assert_eq!(
            s,
            "#!/bin/sh\n\
             # claude-garage statusline wrapper — auto-generated, do not hand-edit.\n\
             # Reinstall (POST /api/statusline/install) regenerates this file.\n\
             INPUT=\"$(cat)\"\n\
             printf '%s' \"$INPUT\" | curl -fsS --max-time 1 -X POST 'http://127.0.0.1:4747/api/statusline/claude?token=tok' -H \"Content-Type: application/json\" --data-binary @- >/dev/null 2>&1 &\n\
             printf '%s' \"$INPUT\" | sh -c 'echo '\\''hi'\\'''\n"
        );
    }
}
