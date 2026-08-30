//! Typed views over the daemon's JSON payloads (port of
//! `tui/lib/api/models.dart`).
//!
//! Shapes mirror `daemon/src/sessions.js` (GET /api/sessions — live entries
//! plus restorable placeholders, both carrying `status`/`since`/`message`/
//! `branch`) and `daemon/src/workspaces.js` (GET /api/workspaces). The TUI is
//! a thin client: nothing here interprets state, it only types it.

use serde_json::Value;

fn as_epoch_ms(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        _ => None,
    }
}

fn as_string(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_owned)
}

/// One registered workspace from `GET /api/workspaces`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceInfo {
    pub name: String,
    pub dir: Option<String>,
    pub branch: Option<String>,
}

impl WorkspaceInfo {
    /// `None` when the entry lacks the required `name` (the Dart port threw).
    pub fn from_json(json: &Value) -> Option<WorkspaceInfo> {
        Some(WorkspaceInfo {
            name: as_string(json.get("name"))?,
            dir: as_string(json.get("dir")),
            branch: as_string(json.get("branch")),
        })
    }
}

/// One session entry from `GET /api/sessions` (live or restorable).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: String,
    pub workspace: String,
    pub label: String,
    /// The session's starting dir (tmux `session_path`, frozen at creation) —
    /// for a worktree session this is the worktree path, not the registered
    /// workspace dir.
    pub dir: Option<String>,
    pub attached: bool,
    /// `needs-input | working | done | idle | restorable`.
    pub status: String,
    /// Epoch ms the current status began (daemon falls back to tmux creation
    /// time for never-transitioned sessions; `None` only for restorable
    /// entries).
    pub since: Option<i64>,
    /// The Notification hook's text; non-`None` only while `needs-input`.
    pub message: Option<String>,
    pub branch: Option<String>,
    pub restorable: bool,
}

impl SessionInfo {
    pub fn from_json(json: &Value) -> Option<SessionInfo> {
        Some(SessionInfo {
            id: as_string(json.get("id"))?,
            workspace: as_string(json.get("workspace"))?,
            label: as_string(json.get("label"))?,
            dir: as_string(json.get("dir")),
            attached: json.get("attached") == Some(&Value::Bool(true)),
            status: as_string(json.get("status")).unwrap_or_else(|| "idle".to_owned()),
            since: as_epoch_ms(json.get("since")),
            message: as_string(json.get("message")),
            branch: as_string(json.get("branch")),
            restorable: json.get("restorable") == Some(&Value::Bool(true)),
        })
    }
}

/// `POST /api/sessions` 201 body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnedSession {
    pub id: String,
    pub workspace: String,
    pub label: String,
    pub dir: Option<String>,
    /// Whether the daemon created an isolated git worktree for this session.
    pub worktree: bool,
}

impl SpawnedSession {
    pub fn from_json(json: &Value) -> Option<SpawnedSession> {
        Some(SpawnedSession {
            id: as_string(json.get("id"))?,
            workspace: as_string(json.get("workspace"))?,
            label: as_string(json.get("label"))?,
            dir: as_string(json.get("dir")),
            worktree: json.get("worktree").is_some_and(|v| !v.is_null()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_info_parses_daemon_shape_with_defaults() {
        let v = json!({"id": "garage/a/x", "workspace": "a", "label": "x"});
        let s = SessionInfo::from_json(&v).unwrap();
        assert_eq!(s.status, "idle");
        assert_eq!(s.since, None);
        assert!(!s.attached);
        assert!(!s.restorable);
    }

    #[test]
    fn since_accepts_any_json_number() {
        let v = json!({"id": "i", "workspace": "w", "label": "l", "since": 123.0});
        assert_eq!(SessionInfo::from_json(&v).unwrap().since, Some(123));
    }
}
