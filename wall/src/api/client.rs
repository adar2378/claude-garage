//! HTTP client for the garage daemon (port of `tui/lib/api/client.dart` +
//! the health gate from `tui/lib/bootstrap.dart`).
//!
//! Plain blocking HTTP against 127.0.0.1 — no auth: the daemon allows
//! no-Origin localhost requests. Runs inside `spawn_blocking` tasks under the
//! tokio runtime. SSE lives in `sse.rs`; this file is request/response only.

use std::fmt;
use std::time::Duration;

use serde_json::{Value, json};

use super::models::{
    FinishAction, HooksInstallResult, RestartResponse, SessionInfo, SpawnedSession, WorkspaceInfo,
    WorktreeRecord,
};

/// Pure port-selection logic behind [`garage_daemon_port`], split out so it
/// unit-tests without touching the process environment.
pub fn port_from_env(tui_port: Option<&str>, garage_port: Option<&str>) -> u16 {
    for value in [tui_port, garage_port].into_iter().flatten() {
        if let Ok(port) = value.trim().parse::<u32>()
            && port > 0
            && port <= 65535
        {
            return port as u16;
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

/// Whole-request timeout for [`GarageClient::finish_worktree`].
const FINISH_WORKTREE_TIMEOUT: Duration = Duration::from_secs(120);

/// Whole-request timeout for [`GarageClient::pick_directory`]: the daemon's
/// 120 s osascript timeout (daemon/src/picker.js) plus headroom, so the
/// daemon's own timeout reply always wins the race.
const PICK_DIRECTORY_TIMEOUT: Duration = Duration::from_secs(130);

pub struct GarageClient {
    pub base_url: String,
    agent: ureq::Agent,
}

impl GarageClient {
    pub fn new(base_url: Option<String>) -> GarageClient {
        GarageClient {
            base_url: base_url.unwrap_or_else(default_daemon_base_url),
            agent: ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(5)))
                .timeout_per_read(Some(Duration::from_secs(15)))
                .timeout_per_write(Some(Duration::from_secs(15)))
                // Non-2xx answers are replies to read (see `read_json`), not errors.
                .http_status_as_error(false)
                .build()
                .into(),
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

    /// `POST /api/statusline/install` (spec tui-context-meters "Install
    /// affordance") — installs the chaining statusline wrapper into
    /// `~/.claude/settings.json`. The caller only needs success/failure; the
    /// strip notice text is static success wording or the daemon's own
    /// error message (via [`ApiError`]).
    pub fn install_statusline(&self) -> Result<(), ApiError> {
        self.post_json("/api/statusline/install", &json!({}))?;
        Ok(())
    }

    /// `POST /api/hooks/install` (spec tui-hooks-install) — merges garage's
    /// Claude Code hooks into `~/.claude/settings.json` (idempotent; backup
    /// and atomic write daemon-side). A corrupt settings file comes back as
    /// a 422 whose `error` lands in [`ApiError::Status`].
    pub fn install_hooks(&self) -> Result<HooksInstallResult, ApiError> {
        let body = self.post_json("/api/hooks/install", &json!({}))?;
        Ok(body
            .as_ref()
            .map(HooksInstallResult::from_json)
            .unwrap_or_default())
    }

    /// `POST /api/worktrees/finish {worktree, action}` (spec
    /// tui-worktree-finish) — merge or discard the worktree a closed session
    /// left behind. A refusal (dirty worktree, merge conflict — 409) comes
    /// back as [`ApiError::Status`] carrying the daemon's `error`, which is
    /// already git's trimmed stderr for a conflict.
    pub fn finish_worktree(
        &self,
        record: &WorktreeRecord,
        action: FinishAction,
    ) -> Result<(), ApiError> {
        // A merge on a large repo can outlast the agent's 15 s read timeout;
        // timing out would report failure for a merge that actually landed.
        read_json(
            self.agent
                .post(&format!("{}/api/worktrees/finish", self.base_url))
                .config()
                .timeout_global(Some(FINISH_WORKTREE_TIMEOUT))
                .build()
                .header("Content-Type", "application/json")
                .send(
                    json!({ "worktree": record.to_json(), "action": action.as_str() }).to_string(),
                ),
        )?;
        Ok(())
    }

    /// `POST /api/sessions`. With `worktree` the daemon spawns into an
    /// isolated git worktree.
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
    /// notifications stay suppressed while the TUI is visible (spec
    /// tui-triage "Off-screen escalation").
    pub fn post_visibility(&self, client_id: &str, visible: bool) -> Result<(), ApiError> {
        self.post_json(
            "/api/ui/visibility",
            &json!({ "clientId": client_id, "visible": visible }),
        )?;
        Ok(())
    }

    /// `POST /api/sessions/restore {id}` — restore one restorable session
    /// (restore-all is the caller issuing parallel per-id calls). Returns the
    /// failure reason for this id when the daemon reports one, `None` on
    /// success.
    pub fn restore_session(&self, id: &str) -> Result<Option<String>, ApiError> {
        let body = self.post_json("/api/sessions/restore", &json!({ "id": id }))?;
        let failed = body
            .as_ref()
            .and_then(|b| b.get("failed"))
            .and_then(Value::as_array);
        if let Some(failed) = failed
            && let Some(first) = failed.first()
        {
            let reason = first
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("restore failed");
            return Ok(Some(reason.to_owned()));
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
        let body = self.post_json(
            "/api/sessions/restart",
            &json!({ "id": id, "force": force }),
        )?;
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
    /// worktree record from the response (`{path, branch, repoDir, target}`)
    /// or `None` for a plain session.
    pub fn delete_session(
        &self,
        id: &str,
        meta_only: bool,
    ) -> Result<Option<WorktreeRecord>, ApiError> {
        let path = format!(
            "/api/sessions/{}{}",
            encode_component(id),
            if meta_only { "?meta=1" } else { "" }
        );
        let body = read_json(
            self.agent
                .delete(&format!("{}{}", self.base_url, path))
                .call(),
        )?;
        Ok(body
            .as_ref()
            .and_then(|b| b.get("worktree"))
            .and_then(WorktreeRecord::from_json))
    }

    /// `PUT /api/workspaces {name, dir}` — register a workspace.
    pub fn put_workspace(&self, name: &str, dir: &str) -> Result<(), ApiError> {
        read_json(
            self.agent
                .put(&format!("{}/api/workspaces", self.base_url))
                .header("Content-Type", "application/json")
                .send(json!({ "name": name, "dir": dir }).to_string()),
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
        let body = read_json(
            self.agent
                .delete(&format!("{}{}", self.base_url, path))
                .call(),
        )?;
        Ok(body.filter(|b| b.is_object()))
    }

    /// `POST /api/pick-directory` — the daemon opens the native macOS
    /// "choose folder" dialog and blocks until the user answers. `Some(dir)`
    /// on a pick, `None` on cancel; a non-macOS daemon answers 501 (surfaced
    /// as [`ApiError::Status`]). The dialog may legitimately sit open for
    /// the daemon's full 120 s osascript timeout, so this one request
    /// overrides the agent's 15 s read timeout.
    pub fn pick_directory(&self) -> Result<Option<String>, ApiError> {
        let body = read_json(
            self.agent
                .post(&format!("{}/api/pick-directory", self.base_url))
                .config()
                .timeout_global(Some(PICK_DIRECTORY_TIMEOUT))
                .build()
                .header("Content-Type", "application/json")
                .send("{}"),
        )?;
        let body = body.ok_or_else(|| ApiError::Transport("empty picker reply".into()))?;
        if let Some(dir) = body.get("dir").and_then(Value::as_str) {
            return Ok(Some(dir.to_owned()));
        }
        if body.get("cancelled").and_then(Value::as_bool) == Some(true) {
            return Ok(None);
        }
        Err(ApiError::Transport(
            "unexpected picker response shape".into(),
        ))
    }

    /// `GET /api/attach?id=` — how to attach a tile to this session on the
    /// daemon's backend.
    pub fn fetch_attach_spec(&self, id: &str) -> Result<crate::attach::AttachSpec, ApiError> {
        let body = self
            .get_json(&format!("/api/attach?id={}", encode_component(id)))?
            .ok_or_else(|| ApiError::Transport("empty attach reply".into()))?;
        serde_json::from_value(body)
            .map_err(|e| ApiError::Transport(format!("bad attach spec: {e}")))
    }

    // ── Plugin host (p24) ────────────────────────────────────────────────

    /// `GET /api/plugins`: the installed plugins with their actions, buttons
    /// and panes.
    pub fn fetch_plugins(&self) -> Result<garage_protocol::plugins::PluginList, ApiError> {
        let body = self
            .get_json("/api/plugins")?
            .ok_or_else(|| ApiError::Transport("empty plugin list".into()))?;
        serde_json::from_value(body)
            .map_err(|e| ApiError::Transport(format!("bad plugin list: {e}")))
    }

    /// `GET /api/plugins/widgets[?plugin=]`: only the named plugin's content
    /// when `plugin` is given.
    pub fn fetch_plugin_widgets(
        &self,
        plugin: Option<&str>,
    ) -> Result<garage_protocol::plugins::WidgetsSnapshot, ApiError> {
        let path = match plugin {
            Some(p) => format!("/api/plugins/widgets?plugin={}", encode_component(p)),
            None => "/api/plugins/widgets".to_owned(),
        };
        let body = self
            .get_json(&path)?
            .ok_or_else(|| ApiError::Transport("empty widget snapshot".into()))?;
        serde_json::from_value(body)
            .map_err(|e| ApiError::Transport(format!("bad widget snapshot: {e}")))
    }

    /// `GET /api/plugins/panes`.
    pub fn fetch_plugin_panes(&self) -> Result<garage_protocol::plugins::PanesResponse, ApiError> {
        let body = self
            .get_json("/api/plugins/panes")?
            .ok_or_else(|| ApiError::Transport("empty pane list".into()))?;
        serde_json::from_value(body).map_err(|e| ApiError::Transport(format!("bad pane list: {e}")))
    }

    /// `POST /api/plugins/{id}/actions/{action}/run` with the UI token.
    pub fn run_plugin_action(
        &self,
        plugin: &str,
        action: &str,
        context: &Value,
    ) -> Result<(), ApiError> {
        let path = format!(
            "/api/plugins/{}/actions/{}/run",
            encode_component(plugin),
            encode_component(action)
        );
        self.ui_request("POST", &path, Some(context)).map(|_| ())
    }

    /// `POST /api/plugins/{id}/panes/{pane}/open` with the UI token.
    pub fn open_plugin_pane(
        &self,
        plugin: &str,
        pane: &str,
        context: &Value,
    ) -> Result<(), ApiError> {
        let path = format!(
            "/api/plugins/{}/panes/{}/open",
            encode_component(plugin),
            encode_component(pane)
        );
        self.ui_request("POST", &path, Some(context)).map(|_| ())
    }

    /// `DELETE /api/plugins/{id}/panes/{pane}` with the UI token.
    pub fn close_plugin_pane(&self, plugin: &str, pane: &str) -> Result<(), ApiError> {
        let path = format!(
            "/api/plugins/{}/panes/{}",
            encode_component(plugin),
            encode_component(pane)
        );
        self.ui_request("DELETE", &path, None).map(|_| ())
    }

    /// `PUT /api/plugins/ui/slots` with the UI token.
    pub fn put_ui_slots(&self, geometry: &Value) -> Result<(), ApiError> {
        self.ui_request("PUT", "/api/plugins/ui/slots", Some(geometry))
            .map(|_| ())
    }

    /// A request that carries the UI token. The token is re-read from disk
    /// once when the daemon answers 401 (it recreates the file only if it was
    /// gone), and the call retried a single time.
    fn ui_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>, ApiError> {
        let tokens = UiToken::global();
        for attempt in 0..2 {
            let token = tokens
                .get()
                .map_err(|e| ApiError::Transport(format!("no UI token: {e}")))?;
            let request = ureq::http::Request::builder()
                .method(method)
                .uri(format!("{}{}", self.base_url, path))
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(body.map_or_else(|| "{}".to_owned(), Value::to_string))
                .map_err(|e| ApiError::Transport(e.to_string()))?;
            let result = read_json(self.agent.run(request));
            if attempt == 0 && matches!(result, Err(ApiError::Status { status: 401, .. })) {
                tokens.forget();
                continue;
            }
            return result;
        }
        unreachable!("the loop returns on its second pass")
    }

    fn get_json(&self, path: &str) -> Result<Option<Value>, ApiError> {
        read_json(self.agent.get(&format!("{}{}", self.base_url, path)).call())
    }

    fn post_json(&self, path: &str, body: &Value) -> Result<Option<Value>, ApiError> {
        read_json(
            self.agent
                .post(&format!("{}{}", self.base_url, path))
                .header("Content-Type", "application/json")
                .send(body.to_string()),
        )
    }
}

/// The per-install UI token (`<GARAGE_DIR>/ui-token`): sent on the plugin
/// routes only the TUI may call, read once and cached. It is never put into a
/// run context or forwarded anywhere else.
pub struct UiToken {
    path: std::path::PathBuf,
    cached: std::sync::Mutex<Option<String>>,
}

impl UiToken {
    pub fn at(path: std::path::PathBuf) -> UiToken {
        UiToken {
            path,
            cached: std::sync::Mutex::new(None),
        }
    }

    pub fn global() -> &'static UiToken {
        static TOKEN: std::sync::OnceLock<UiToken> = std::sync::OnceLock::new();
        TOKEN.get_or_init(|| UiToken::at(crate::attach::garage_dir().join("ui-token")))
    }

    pub fn get(&self) -> std::io::Result<String> {
        let mut cached = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = cached.as_ref() {
            return Ok(t.clone());
        }
        let token = std::fs::read_to_string(&self.path)?.trim().to_owned();
        if token.is_empty() {
            return Err(std::io::Error::other("the ui-token file is empty"));
        }
        *cached = Some(token.clone());
        Ok(token)
    }

    /// Drop the cached value so the next [`UiToken::get`] re-reads the file.
    pub fn forget(&self) {
        *self.cached.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
fn read_json(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Option<Value>, ApiError> {
    let mut response = result.map_err(|e| ApiError::Transport(e.to_string()))?;
    let status = response.status();
    let text = response.body_mut().read_to_string();
    if status.is_success() {
        let text = text.map_err(|e| ApiError::Transport(e.to_string()))?;
        if text.is_empty() {
            return Ok(None);
        }
        return serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| ApiError::Transport(format!("bad JSON from daemon: {e}")));
    }
    let text = text.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or(text);
    Err(ApiError::Status {
        status: status.as_u16(),
        message,
    })
}

/// True when the daemon answers `GET /api/health` with a 2xx.
pub fn daemon_healthy(base_url: &str, timeout: Duration) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(timeout))
        .timeout_per_read(Some(timeout))
        .build()
        .into();
    // `call` errors on any non-2xx status (the default), so `is_ok` is a 2xx.
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
    fn the_ui_token_is_trimmed_cached_and_re_read_after_forget() {
        let dir = std::env::temp_dir().join(format!("garage-ui-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ui-token");
        let tokens = UiToken::at(path.clone());
        assert!(tokens.get().is_err(), "no file yet");
        std::fs::write(&path, "abc123\n").unwrap();
        assert_eq!(tokens.get().unwrap(), "abc123");
        std::fs::write(&path, "rotated").unwrap();
        assert_eq!(
            tokens.get().unwrap(),
            "abc123",
            "cached until a 401 says otherwise"
        );
        tokens.forget();
        assert_eq!(tokens.get().unwrap(), "rotated");
        std::fs::write(&path, "  \n").unwrap();
        tokens.forget();
        assert!(tokens.get().is_err(), "an empty file is no credential");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn encode_component_escapes_separators() {
        assert_eq!(encode_component("garage/a/x"), "garage%2Fa%2Fx");
        assert_eq!(encode_component("plain-name_1.2~x"), "plain-name_1.2~x");
    }
}
