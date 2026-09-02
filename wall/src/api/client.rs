//! HTTP client for the garage daemon (port of `tui/lib/api/client.dart` +
//! the health gate from `tui/lib/bootstrap.dart`).
//!
//! Plain blocking HTTP against 127.0.0.1 — no auth: the daemon allows
//! no-Origin localhost requests. Runs inside `spawn_blocking` tasks under the
//! tokio runtime. SSE lives in `sse.rs`; this file is request/response only.

use std::fmt;
use std::time::Duration;

use serde_json::{json, Value};

use super::models::{RestartResponse, SessionInfo, SpawnedSession, UsageInfo, WorkspaceInfo};

/// Pure port-selection logic behind [`garage_daemon_port`], split out so it
/// unit-tests without touching the process environment.
pub fn port_from_env(tui_port: Option<&str>, garage_port: Option<&str>) -> u16 {
    for value in [tui_port, garage_port].into_iter().flatten() {
        if let Ok(port) = value.trim().parse::<u32>() {
            if port > 0 && port <= 65535 {
                return port as u16;
            }
        }
    }
    4747
}

/// The daemon port, honoring `GARAGE_TUI_PORT` then `GARAGE_PORT` (the same
/// env the daemon itself reads — see daemon/src/index.js) so the TUI can be
/// pointed at a scratch daemon for testing. Defaults to 4747.
pub fn garage_daemon_port() -> u16 {
    let tui = std::env::var("GARAGE_TUI_PORT").ok();
    let garage = std::env::var("GARAGE_PORT").ok();
    port_from_env(tui.as_deref(), garage.as_deref())
}

/// Base URL for the daemon (also used by the health gate).
pub fn default_daemon_base_url() -> String {
    format!("http://127.0.0.1:{}", garage_daemon_port())
}

/// A failed daemon call: either a non-2xx response (carrying the daemon's
/// `error` body when present, like the Dart `GarageApiException`) or a
/// transport-level failure.
#[derive(Debug)]
pub enum ApiError {
    Status { status: u16, message: String },
    Transport(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Status { status, message } => {
                write!(f, "GarageApiError({status}): {message}")
            }
            ApiError::Transport(message) => write!(f, "GarageApiError(transport): {message}"),
        }
    }
}

impl std::error::Error for ApiError {}

/// Percent-encode a URL path component (ids and workspace names may carry
/// `/`, spaces, …). Everything but unreserved characters is encoded.
fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub struct GarageClient {
    pub base_url: String,
    agent: ureq::Agent,
}

impl GarageClient {
    pub fn new(base_url: Option<String>) -> GarageClient {
        GarageClient {
            base_url: base_url.unwrap_or_else(default_daemon_base_url),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout_read(Duration::from_secs(15))
                .timeout_write(Duration::from_secs(15))
                .build(),
        }
    }

    pub fn fetch_sessions(&self) -> Result<Vec<SessionInfo>, ApiError> {
        let body = self.get_json("/api/sessions")?;
        parse_list(body.as_ref(), SessionInfo::from_json, "sessions")
    }

    pub fn fetch_workspaces(&self) -> Result<Vec<WorkspaceInfo>, ApiError> {
        let body = self.get_json("/api/workspaces")?;
        parse_list(body.as_ref(), WorkspaceInfo::from_json, "workspaces")
    }

    /// `GET /api/usage` (spec tui-context-meters "Strip usage chip") —
    /// always a 200 with both windows possibly null; a missing/malformed
    /// body degrades to the all-null default rather than an error, since a
    /// transient hiccup here should just skip a poll tick, not disrupt the
    /// wall.
    pub fn fetch_usage(&self) -> Result<UsageInfo, ApiError> {
        let body = self.get_json("/api/usage")?;
        Ok(body.as_ref().map(UsageInfo::from_json).unwrap_or_default())
    }

    /// `POST /api/statusline/install` (spec tui-context-meters "Install
    /// affordance") — installs the chaining statusline wrapper into
    /// `~/.claude/settings.json`. The caller only needs success/failure; the
    /// strip notice text is static success wording or the daemon's own
    /// error message (via [`ApiError`]).
    pub fn install_statusline(&self) -> Result<(), ApiError> {
        self.post_json("/api/statusline/install", &json!({}))?;
        Ok(())
    }

    /// `POST /api/sessions`. With `worktree` the daemon spawns into an
    /// isolated git worktree (same contract as the web UI).
    pub fn spawn_session(
        &self,
        workspace: &str,
        label: &str,
        worktree: bool,
    ) -> Result<SpawnedSession, ApiError> {
        let mut payload = json!({ "workspace": workspace, "label": label });
        if worktree {
            payload["worktree"] = Value::Bool(true);
        }
        let body = self.post_json("/api/sessions", &payload)?;
        body.as_ref()
            .and_then(SpawnedSession::from_json)
            .ok_or_else(|| ApiError::Transport("unexpected spawn response shape".into()))
    }

    /// Visibility heartbeat (`POST /api/ui/visibility`) so daemon-side macOS
    /// notifications stay suppressed while the TUI is visible — same contract
    /// as the web UI (spec tui-triage "Off-screen escalation").
    pub fn post_visibility(&self, client_id: &str, visible: bool) -> Result<(), ApiError> {
        self.post_json(
            "/api/ui/visibility",
            &json!({ "clientId": client_id, "visible": visible }),
        )?;
        Ok(())
    }

    /// `POST /api/sessions/restore {id}` — restore one restorable session
    /// (same per-id contract the web UI uses; restore-all is the caller
    /// issuing parallel per-id calls). Returns the failure reason for this id
    /// when the daemon reports one, `None` on success.
    pub fn restore_session(&self, id: &str) -> Result<Option<String>, ApiError> {
        let body = self.post_json("/api/sessions/restore", &json!({ "id": id }))?;
        let failed = body.as_ref().and_then(|b| b.get("failed")).and_then(Value::as_array);
        if let Some(failed) = failed {
            if let Some(first) = failed.first() {
                let reason = first
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("restore failed");
                return Ok(Some(reason.to_owned()));
            }
        }
        Ok(None)
    }

    /// `POST /api/sessions/restart {id, force}` (p16-restart, spec restart
    /// "Restart sessions in place") — respawns the session's tmux pane on
    /// `claude --resume <claudeSessionId>`, keeping the tmux session, the
    /// tile and the title. `force` restarts a `working`/`needs-input`
    /// session the daemon would otherwise skip (design.md D2). Restart-all
    /// is the caller issuing one call per id — the daemon has no workspace
    /// filter — exactly like restore-all.
    pub fn restart_session(&self, id: &str, force: bool) -> Result<RestartResponse, ApiError> {
        let body = self.post_json("/api/sessions/restart", &json!({ "id": id, "force": force }))?;
        body.as_ref()
            .and_then(RestartResponse::from_json)
            .ok_or_else(|| ApiError::Transport("unexpected restart response shape".into()))
    }

    /// `POST /api/daemon/restart` → the successor daemon's pid (design.md
    /// D3). The daemon replies 202 and then drops this connection; nothing
    /// here waits for the successor — the SSE task's existing reconnect
    /// backoff carries the wall across the handoff.
    pub fn restart_daemon(&self) -> Result<u32, ApiError> {
        let body = self.post_json("/api/daemon/restart", &json!({}))?;
        body.as_ref()
            .and_then(|b| b.get("pid"))
            .and_then(Value::as_u64)
            .map(|pid| pid as u32)
            .ok_or_else(|| ApiError::Transport("daemon restart reply carried no pid".into()))
    }

    /// `DELETE /api/sessions/<id>` — kill a live session (and drop its resume
    /// metadata). With `meta_only` (`?meta=1`) only the stored resume metadata
    /// of a NON-live (restorable) session is dropped — the daemon returns 404
    /// for a plain DELETE of a session with no live tmux match. Returns the
    /// worktree record from the response (`{path, branch, repoDir}`) or `None`.
    pub fn delete_session(&self, id: &str, meta_only: bool) -> Result<Option<Value>, ApiError> {
        let path = format!(
            "/api/sessions/{}{}",
            encode_component(id),
            if meta_only { "?meta=1" } else { "" }
        );
        let body = read_json(self.agent.delete(&format!("{}{}", self.base_url, path)).call())?;
        Ok(body
            .as_ref()
            .and_then(|b| b.get("worktree"))
            .filter(|w| w.is_object())
            .cloned())
    }

    /// `PUT /api/workspaces {name, dir}` — register a workspace (same
    /// contract as the web UI's putWorkspace).
    pub fn put_workspace(&self, name: &str, dir: &str) -> Result<(), ApiError> {
        read_json(
            self.agent
                .put(&format!("{}/api/workspaces", self.base_url))
                .set("Content-Type", "application/json")
                .send_string(&json!({ "name": name, "dir": dir }).to_string()),
        )?;
        Ok(())
    }

    /// `DELETE /api/workspaces/<name>` — registry-only removal by default
    /// (the TUI's `X`-`X` confirm must not touch tmux: live sessions keep
    /// running and reappear in the rail as an unregistered group after the
    /// refetch; the 204 yields `None`). With `kill_sessions` (`?sessions=kill`,
    /// the p8.4 `X` then `K` confirm) the daemon kills every live
    /// `garage/<name>/*` tmux session first and returns
    /// `{removed, killedSessions, failedSessions?}` — returned here for the
    /// strip notice.
    pub fn remove_workspace(
        &self,
        name: &str,
        kill_sessions: bool,
    ) -> Result<Option<Value>, ApiError> {
        let path = format!(
            "/api/workspaces/{}{}",
            encode_component(name),
            if kill_sessions { "?sessions=kill" } else { "" }
        );
        let body = read_json(self.agent.delete(&format!("{}{}", self.base_url, path)).call())?;
        Ok(body.filter(|b| b.is_object()))
    }

    fn get_json(&self, path: &str) -> Result<Option<Value>, ApiError> {
        read_json(self.agent.get(&format!("{}{}", self.base_url, path)).call())
    }

    fn post_json(&self, path: &str, body: &Value) -> Result<Option<Value>, ApiError> {
        read_json(
            self.agent
                .post(&format!("{}{}", self.base_url, path))
                .set("Content-Type", "application/json")
                .send_string(&body.to_string()),
        )
    }
}

fn parse_list<T>(
    body: Option<&Value>,
    parse: impl Fn(&Value) -> Option<T>,
    what: &str,
) -> Result<Vec<T>, ApiError> {
    let entries = body
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Transport(format!("unexpected {what} payload")))?;
    entries
        .iter()
        .map(|entry| {
            parse(entry).ok_or_else(|| ApiError::Transport(format!("malformed {what} entry")))
        })
        .collect()
}

/// Shared response handling (port of the Dart `_readJson`): non-2xx becomes
/// [`ApiError::Status`] carrying the daemon's `error` body when present; an
/// empty body yields `None`.
fn read_json(result: Result<ureq::Response, ureq::Error>) -> Result<Option<Value>, ApiError> {
    match result {
        Ok(response) => {
            let text = response
                .into_string()
                .map_err(|e| ApiError::Transport(e.to_string()))?;
            if text.is_empty() {
                return Ok(None);
            }
            serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| ApiError::Transport(format!("bad JSON from daemon: {e}")))
        }
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or(text);
            Err(ApiError::Status { status, message })
        }
        Err(e) => Err(ApiError::Transport(e.to_string())),
    }
}

/// True when the daemon answers `GET /api/health` with a 2xx.
pub fn daemon_healthy(base_url: &str, timeout: Duration) -> bool {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(timeout)
        .timeout_read(timeout)
        .build();
    agent.get(&format!("{base_url}/api/health")).call().is_ok()
}

/// The actionable health-gate error (spec packaging: actionable error naming
/// `npx claude-garage`). Split from [`require_daemon`] for testability.
pub fn daemon_unreachable_message(base_url: &str) -> String {
    format!(
        "claude-garage daemon is not reachable at {base_url}.\n\
         Start it first:\n\
         \n\
         \x20 npx claude-garage\n\
         \n\
         then run the TUI again."
    )
}

/// Health-gate the UI: print an actionable error and exit(1) when the daemon
/// is unreachable. Call before touching the terminal.
pub fn require_daemon(base_url: &str) {
    if daemon_healthy(base_url, Duration::from_secs(2)) {
        return;
    }
    eprintln!("{}", daemon_unreachable_message(base_url));
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_honors_garage_tui_port_then_garage_port_then_default() {
        assert_eq!(port_from_env(Some("5050"), Some("6060")), 5050);
        assert_eq!(port_from_env(None, Some("6060")), 6060);
        assert_eq!(port_from_env(None, None), 4747);
    }

    #[test]
    fn invalid_ports_fall_through() {
        assert_eq!(port_from_env(Some("nope"), Some("6060")), 6060);
        assert_eq!(port_from_env(Some("0"), None), 4747);
        assert_eq!(port_from_env(Some("70000"), None), 4747);
    }

    #[test]
    fn error_message_names_npx_claude_garage() {
        let msg = daemon_unreachable_message("http://127.0.0.1:4747");
        assert!(msg.contains("npx claude-garage"));
        assert!(msg.contains("http://127.0.0.1:4747"));
    }

    #[test]
    fn encode_component_escapes_separators() {
        assert_eq!(encode_component("garage/a/x"), "garage%2Fa%2Fx");
        assert_eq!(encode_component("plain-name_1.2~x"), "plain-name_1.2~x");
    }
}
