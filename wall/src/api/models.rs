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

/// A JSON number → a 0-100 percentage, clamped then rounded. The daemon
/// already clamps (`clampPercentage`/transcript's `Math.round`), but
/// defensive clamping here costs nothing and keeps `context_segments`
/// (ui/tile.rs) safe even against a malformed payload.
fn as_percentage(v: Option<&Value>) -> Option<u32> {
    match v {
        Some(Value::Number(n)) => n.as_f64().map(|f| f.clamp(0.0, 100.0).round() as u32),
        _ => None,
    }
}

/// `resetsAt` rides through as whatever JSON scalar the statusline payload
/// carried (an ISO string in practice) — the wall never interprets it, only
/// carries it, so a string or a number are both accepted verbatim.
fn as_resets_at(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// One session's `context` entry (`GET /api/sessions` — see
/// `daemon/src/sessions.js`'s `{usedPercentage, source: "statusline"|
/// "transcript"} | null` shape; proposal.md "API").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextInfo {
    /// 0-100; the daemon already clamps/rounds this.
    pub used_percentage: u32,
    /// `"statusline"` or `"transcript"` — see the context-telemetry spec.
    pub source: String,
}

impl ContextInfo {
    /// `None` when `usedPercentage` is missing or not a number (the Dart
    /// port's "throw on malformed" — treated here as "no context", same as
    /// the field being absent).
    pub fn from_json(json: &Value) -> Option<ContextInfo> {
        Some(ContextInfo {
            used_percentage: as_percentage(json.get("usedPercentage"))?,
            source: as_string(json.get("source")).unwrap_or_default(),
        })
    }
}

/// One window of `GET /api/usage` (`{usedPercentage, resetsAt} | null`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageWindow {
    pub used_percentage: u32,
    pub resets_at: Option<String>,
}

impl UsageWindow {
    pub fn from_json(json: &Value) -> Option<UsageWindow> {
        Some(UsageWindow {
            used_percentage: as_percentage(json.get("usedPercentage"))?,
            resets_at: as_resets_at(json.get("resetsAt")),
        })
    }
}

/// `GET /api/usage` response (`daemon/src/sessions.js`'s
/// `{fiveHour: {...}|null, sevenDay: {...}|null}` — account-wide, from the
/// most recent statusline post; both null until one arrives).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct UsageInfo {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
}

impl UsageInfo {
    /// Never fails — a missing/malformed body degrades to "both null" so a
    /// transient parse hiccup never blocks the strip (it just shows no chip
    /// this tick).
    pub fn from_json(json: &Value) -> UsageInfo {
        UsageInfo {
            five_hour: json
                .get("fiveHour")
                .filter(|v| !v.is_null())
                .and_then(UsageWindow::from_json),
            seven_day: json
                .get("sevenDay")
                .filter(|v| !v.is_null())
                .and_then(UsageWindow::from_json),
        }
    }
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
    /// The daemon-normalized pane/OSC title (p10 `list-panes -F
    /// #{pane_title}`; empty/hostname/shell-name already normalized to
    /// `null` server-side) — `None` for restorable entries and for any live
    /// session with nothing meaningful to show.
    pub title: Option<String>,
    /// p11: `{usedPercentage, source: "statusline"|"transcript"} | null` —
    /// `None` for a session with no context data yet (or a restorable
    /// entry, which never carries one; see context-telemetry spec).
    pub context: Option<ContextInfo>,
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
            title: as_string(json.get("title")),
            context: json
                .get("context")
                .filter(|v| !v.is_null())
                .and_then(ContextInfo::from_json),
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
        assert_eq!(s.title, None);
        assert_eq!(s.context, None);
    }

    // ── p11: context (spec tui-context-meters) ───────────────────────────

    #[test]
    fn context_parses_the_daemon_shape() {
        let v = json!({
            "id": "i", "workspace": "w", "label": "l",
            "context": {"usedPercentage": 42.4, "source": "statusline"}
        });
        let ctx = SessionInfo::from_json(&v).unwrap().context.unwrap();
        assert_eq!(ctx.used_percentage, 42);
        assert_eq!(ctx.source, "statusline");
    }

    #[test]
    fn context_null_and_absent_both_map_to_none() {
        let v = json!({"id": "i", "workspace": "w", "label": "l", "context": null});
        assert_eq!(SessionInfo::from_json(&v).unwrap().context, None);
        let v = json!({"id": "i", "workspace": "w", "label": "l"});
        assert_eq!(SessionInfo::from_json(&v).unwrap().context, None);
    }

    #[test]
    fn usage_info_parses_both_windows_and_a_null_one() {
        let v = json!({
            "fiveHour": {"usedPercentage": 24.0, "resetsAt": "2026-01-01T00:00:00Z"},
            "sevenDay": null
        });
        let u = UsageInfo::from_json(&v);
        assert_eq!(u.five_hour.as_ref().unwrap().used_percentage, 24);
        assert_eq!(
            u.five_hour.as_ref().unwrap().resets_at.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        assert_eq!(u.seven_day, None);
    }

    #[test]
    fn usage_info_both_null_is_the_default() {
        let v = json!({"fiveHour": null, "sevenDay": null});
        assert_eq!(UsageInfo::from_json(&v), UsageInfo::default());
    }

    #[test]
    fn title_parses_when_present_and_defaults_to_none() {
        let v = json!({"id": "i", "workspace": "w", "label": "l", "title": "✳ testing subtitle"});
        assert_eq!(
            SessionInfo::from_json(&v).unwrap().title.as_deref(),
            Some("✳ testing subtitle")
        );
        let v = json!({"id": "i", "workspace": "w", "label": "l", "title": null});
        assert_eq!(SessionInfo::from_json(&v).unwrap().title, None);
    }

    #[test]
    fn since_accepts_any_json_number() {
        let v = json!({"id": "i", "workspace": "w", "label": "l", "since": 123.0});
        assert_eq!(SessionInfo::from_json(&v).unwrap().since, Some(123));
    }
}
