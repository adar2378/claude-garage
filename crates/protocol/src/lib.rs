//! Wire types for the claude-garage daemon HTTP API.
//!
//! Source of truth: `openspec/changes/p21-rust-daemon/specs/daemon-api/spec.md`
//! and the JSON the JS daemon (`daemon/src/*.js`) produces. Field names are
//! camelCase on the wire. Optional fields that the JS daemon omits (rather
//! than sending `null`) are `skip_serializing_if = "Option::is_none"`;
//! fields it sends as explicit `null` are plain `Option`s.
//!
//! `wall` still parses payloads by hand (`wall/src/api/models.rs` is lenient
//! by design: it degrades a malformed field to "absent" instead of failing
//! the whole list), so it does not use these types yet.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod api;
pub mod auth;
pub mod plugins;
pub mod widgets;

/// Body of every non-2xx response: `{"error": "<message>", "code":
/// "<stable_snake_code>"}`, plus `"permission"` on a 403 for a missing
/// permission. `error` is for people and may change; `code` is the contract.
/// Routes may add other fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub error: String,
    pub code: String,
    /// The permission the caller lacks (`code` is `permission_denied`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

/// The stable `code` of an [`ErrorBody`]. Codes are API: a plugin matches on
/// them, so one is never renamed or reused for something else in v1. New
/// codes may appear, so match with a catch-all arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCode {
    /// `bad_request`: 400 with nothing more specific to say.
    BadRequest,
    /// `unauthorized`: 401 with nothing more specific to say.
    Unauthorized,
    /// `forbidden`: 403 with nothing more specific to say.
    Forbidden,
    /// `not_found`: 404 with nothing more specific to say.
    NotFound,
    /// `conflict`: 409 with nothing more specific to say.
    Conflict,
    /// `payload_too_large`: The body or a field is over its size limit.
    PayloadTooLarge,
    /// `invalid_request`: 422 with nothing more specific to say.
    InvalidRequest,
    /// `rate_limited`: Too many calls; wait for `Retry-After` seconds.
    RateLimited,
    /// `internal_error`: The daemon failed.
    InternalError,
    /// `not_implemented`: Not available in this build.
    NotImplemented,
    /// `unavailable`: The daemon cannot do this now.
    Unavailable,
    /// `unspecified`: Any other status.
    Unspecified,
    /// `invalid_json`: The body is not valid JSON.
    InvalidJson,
    /// `missing_token`: No `Authorization: Bearer` token.
    MissingToken,
    /// `invalid_token`: The token is not a plugin's token.
    InvalidToken,
    /// `wrong_plugin`: The token belongs to another plugin.
    WrongPlugin,
    /// `ui_token_required`: The route is for the wall; a plugin token is not enough.
    UiTokenRequired,
    /// `plugin_disabled`: The plugin is disabled.
    PluginDisabled,
    /// `permission_denied`: The plugin lacks a permission; `permission` names it.
    PermissionDenied,
    /// `unknown_plugin`: No such plugin.
    UnknownPlugin,
    /// `unknown_widget`: The plugin declares no such widget.
    UnknownWidget,
    /// `invalid_widget`: The widget body is refused.
    InvalidWidget,
    /// `unknown_action`: The plugin has no such action.
    UnknownAction,
    /// `action_not_allowed`: The action is not one a plugin may name.
    ActionNotAllowed,
    /// `invalid_action`: `action` is missing or malformed.
    InvalidAction,
    /// `invalid_target`: `target` names something that does not exist.
    InvalidTarget,
    /// `invalid_text`: `text` is missing or empty.
    InvalidText,
    /// `unknown_pane`: The plugin declares no such pane.
    UnknownPane,
    /// `too_many_panes`: The plugin has too many panes open.
    TooManyPanes,
    /// `pane_failed`: The pane could not be opened or closed.
    PaneFailed,
    /// `unknown_session`: No such session.
    UnknownSession,
    /// `session_not_allowed`: Not a garage session.
    SessionNotAllowed,
    /// `invalid_if_status`: `if_status` is malformed.
    InvalidIfStatus,
    /// `status_blocked`: The session is in a status `if_status` refuses; nothing was typed.
    StatusBlocked,
    /// `send_failed`: The keys could not be sent.
    SendFailed,
    /// `unknown_workspace`: No such workspace.
    UnknownWorkspace,
    /// `host_not_allowed`: The `Host` header is not the daemon's.
    HostNotAllowed,
    /// `origin_not_allowed`: A browser origin tried to write.
    OriginNotAllowed,
    /// `invalid_nonce`: The UI challenge nonce is malformed.
    InvalidNonce,
    /// `no_ui_token`: This daemon has no UI token.
    NoUiToken,
    /// `bundled_plugin`: The plugin ships with garage and cannot be changed this way.
    BundledPlugin,
    /// `reload_failed`: The plugin registry could not be reloaded.
    ReloadFailed,
    /// `unknown_permission`: Not a plugin API v1 permission.
    UnknownPermission,
}

impl ErrorCode {
    /// Every code, in the order the docs list them.
    pub const ALL: &'static [ErrorCode] = &[
        ErrorCode::BadRequest,
        ErrorCode::Unauthorized,
        ErrorCode::Forbidden,
        ErrorCode::NotFound,
        ErrorCode::Conflict,
        ErrorCode::PayloadTooLarge,
        ErrorCode::InvalidRequest,
        ErrorCode::RateLimited,
        ErrorCode::InternalError,
        ErrorCode::NotImplemented,
        ErrorCode::Unavailable,
        ErrorCode::Unspecified,
        ErrorCode::InvalidJson,
        ErrorCode::MissingToken,
        ErrorCode::InvalidToken,
        ErrorCode::WrongPlugin,
        ErrorCode::UiTokenRequired,
        ErrorCode::PluginDisabled,
        ErrorCode::PermissionDenied,
        ErrorCode::UnknownPlugin,
        ErrorCode::UnknownWidget,
        ErrorCode::InvalidWidget,
        ErrorCode::UnknownAction,
        ErrorCode::ActionNotAllowed,
        ErrorCode::InvalidAction,
        ErrorCode::InvalidTarget,
        ErrorCode::InvalidText,
        ErrorCode::UnknownPane,
        ErrorCode::TooManyPanes,
        ErrorCode::PaneFailed,
        ErrorCode::UnknownSession,
        ErrorCode::SessionNotAllowed,
        ErrorCode::InvalidIfStatus,
        ErrorCode::StatusBlocked,
        ErrorCode::SendFailed,
        ErrorCode::UnknownWorkspace,
        ErrorCode::HostNotAllowed,
        ErrorCode::OriginNotAllowed,
        ErrorCode::InvalidNonce,
        ErrorCode::NoUiToken,
        ErrorCode::BundledPlugin,
        ErrorCode::ReloadFailed,
        ErrorCode::UnknownPermission,
    ];

    /// The wire spelling: lower snake_case.
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "bad_request",
            ErrorCode::Unauthorized => "unauthorized",
            ErrorCode::Forbidden => "forbidden",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Conflict => "conflict",
            ErrorCode::PayloadTooLarge => "payload_too_large",
            ErrorCode::InvalidRequest => "invalid_request",
            ErrorCode::RateLimited => "rate_limited",
            ErrorCode::InternalError => "internal_error",
            ErrorCode::NotImplemented => "not_implemented",
            ErrorCode::Unavailable => "unavailable",
            ErrorCode::Unspecified => "unspecified",
            ErrorCode::InvalidJson => "invalid_json",
            ErrorCode::MissingToken => "missing_token",
            ErrorCode::InvalidToken => "invalid_token",
            ErrorCode::WrongPlugin => "wrong_plugin",
            ErrorCode::UiTokenRequired => "ui_token_required",
            ErrorCode::PluginDisabled => "plugin_disabled",
            ErrorCode::PermissionDenied => "permission_denied",
            ErrorCode::UnknownPlugin => "unknown_plugin",
            ErrorCode::UnknownWidget => "unknown_widget",
            ErrorCode::InvalidWidget => "invalid_widget",
            ErrorCode::UnknownAction => "unknown_action",
            ErrorCode::ActionNotAllowed => "action_not_allowed",
            ErrorCode::InvalidAction => "invalid_action",
            ErrorCode::InvalidTarget => "invalid_target",
            ErrorCode::InvalidText => "invalid_text",
            ErrorCode::UnknownPane => "unknown_pane",
            ErrorCode::TooManyPanes => "too_many_panes",
            ErrorCode::PaneFailed => "pane_failed",
            ErrorCode::UnknownSession => "unknown_session",
            ErrorCode::SessionNotAllowed => "session_not_allowed",
            ErrorCode::InvalidIfStatus => "invalid_if_status",
            ErrorCode::StatusBlocked => "status_blocked",
            ErrorCode::SendFailed => "send_failed",
            ErrorCode::UnknownWorkspace => "unknown_workspace",
            ErrorCode::HostNotAllowed => "host_not_allowed",
            ErrorCode::OriginNotAllowed => "origin_not_allowed",
            ErrorCode::InvalidNonce => "invalid_nonce",
            ErrorCode::NoUiToken => "no_ui_token",
            ErrorCode::BundledPlugin => "bundled_plugin",
            ErrorCode::ReloadFailed => "reload_failed",
            ErrorCode::UnknownPermission => "unknown_permission",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `{"ok": true}` acknowledgements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Ack {
    pub ok: bool,
}

/// `GET /api/health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Health {
    pub status: String,
    pub version: Option<String>,
    pub pid: u32,
}

/// Session state. `Restorable` only appears in list entries built from
/// resume metadata; the status store itself never holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SessionStatus {
    Idle,
    Working,
    NeedsInput,
    Done,
    Restorable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContextSource {
    Statusline,
    Transcript,
}

/// `context` of a session list entry. The percentage is clamped to 0-100 by
/// the daemon but may be fractional (statusline) or rounded (transcript).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub used_percentage: f64,
    pub source: ContextSource,
}

/// A worktree as stored in resume metadata and returned by create/restore.
/// `target` (the branch checked out in `repo_dir`) is only present on
/// `DELETE /api/sessions/:id` responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub repo_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// The `{path, branch}` pair create-session answers with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorktreeRef {
    pub path: String,
    pub branch: String,
}

/// One entry of `GET /api/sessions`: live, or restorable (`status` is
/// `restorable`, `restorable` is `true`, no `createdAt`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub workspace: String,
    pub label: String,
    pub dir: Option<String>,
    pub attached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    pub status: SessionStatus,
    pub message: Option<String>,
    pub since: Option<i64>,
    pub branch: Option<String>,
    pub title: Option<String>,
    pub context: Option<Context>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restorable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateSessionRequest {
    pub workspace: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateSessionResponse {
    pub id: String,
    pub workspace: String,
    pub label: String,
    pub dir: String,
    pub worktree: Option<WorktreeRef>,
}

/// `DELETE /api/sessions/:id` (`meta` is only present with `?meta=1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeleteSessionResponse {
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<bool>,
    pub worktree: Option<Worktree>,
}

/// Body of `POST /api/sessions/restore` and `/restart`: `{id}` or `{all}`;
/// `force` is only read by restart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Restored {
    pub id: String,
    pub workspace: String,
    pub label: String,
    pub dir: String,
    pub worktree: Option<Worktree>,
    pub resumed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RestoreFailure {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RestoreResponse {
    pub restored: Vec<Restored>,
    pub failed: Vec<RestoreFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Restarted {
    pub id: String,
    pub resumed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RestartSkipped {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IdError {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RestartResponse {
    pub restarted: Vec<Restarted>,
    pub skipped: Vec<RestartSkipped>,
    pub failed: Vec<IdError>,
}

/// One entry of `GET /api/workspaces`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Workspace {
    pub name: String,
    pub dir: String,
    pub branch: Option<String>,
}

/// `PUT /api/workspaces` body and response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceRef {
    pub name: String,
    pub dir: String,
}

/// `PATCH /api/workspaces/:name` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RenameWorkspaceRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RenamedSession {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RenameWorkspaceResponse {
    pub name: String,
    pub dir: String,
    pub renamed_sessions: Vec<RenamedSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_sessions: Option<Vec<IdError>>,
}

/// `DELETE /api/workspaces/:name?sessions=kill` (plain delete answers 204).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteWorkspaceResponse {
    pub removed: String,
    pub killed_sessions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_sessions: Option<Vec<IdError>>,
}

/// `POST /api/hooks/claude` answers `{ok, applied}` or `{ok, ignored}`
/// (`ignored` is `true` or the string `"idle-reminder"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookAck {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored: Option<Value>,
}

/// `POST /api/hooks/install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HooksInstallResponse {
    pub ok: bool,
    pub installed: bool,
    pub already_installed: bool,
    pub backup: Option<String>,
}

/// `POST /api/statusline/install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatuslineInstallResponse {
    pub ok: bool,
    pub installed: bool,
    pub already_installed: bool,
    pub chained: bool,
    pub backup: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusLineCommand {
    #[serde(rename = "type")]
    pub kind: String,
    pub command: String,
}

/// `GET /api/statusline/snippet`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatuslineSnippet {
    pub status_line: StatusLineCommand,
    pub script: String,
}

/// One rate-limit window of `GET /api/usage`. `resetsAt` is whatever scalar
/// Claude Code's statusline carried (an ISO string in practice), or null.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub used_percentage: f64,
    pub resets_at: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
}

/// `POST /api/ui/visibility` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityRequest {
    pub client_id: String,
    pub visible: bool,
}

/// `POST /api/daemon/restart` answers 202 with the successor's pid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonRestartResponse {
    pub pid: u32,
}

/// `POST /api/open-editor` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpenEditorRequest {
    pub workspace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// `POST /api/pick-directory`: `{dir}` or `{cancelled: true}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PickDirectoryResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FinishAction {
    Merge,
    Discard,
    Keep,
}

/// `POST /api/worktrees/finish` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FinishWorktreeRequest {
    pub worktree: Worktree,
    pub action: FinishAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FinishWorktreeResponse {
    pub action: FinishAction,
    pub path: String,
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discarded: Option<bool>,
}

/// Data of `event: status` on `GET /api/events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusEvent {
    pub id: String,
    pub status: SessionStatus,
    pub since: Option<i64>,
}

/// One Server-Sent Event of `GET /api/events` (the `: connected` and
/// `: keepalive` comment lines are framing, not events).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Status(StatusEvent),
    /// The set of live sessions or a pane title changed (data is `{}`).
    Sessions,
}

impl Event {
    /// The SSE `event:` field.
    pub fn name(&self) -> &'static str {
        match self {
            Event::Status(_) => "status",
            Event::Sessions => "sessions",
        }
    }

    /// The SSE `data:` field (compact JSON).
    pub fn data(&self) -> String {
        match self {
            Event::Status(s) => serde_json::to_string(s).expect("StatusEvent serializes"),
            Event::Sessions => "{}".to_owned(),
        }
    }

    /// A full SSE frame, as `events.js` writes it.
    pub fn frame(&self) -> String {
        format!("event: {}\ndata: {}\n\n", self.name(), self.data())
    }
}

/// How a client attaches a terminal to a session: the seam between the TUI's
/// How the TUI attaches to a session. Deliberately data, not a command line:
/// the TUI builds the fixed command itself, so whatever answers on the daemon
/// port can never make it run an arbitrary program or dial an arbitrary host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AttachSpec {
    /// `tmux [-L <socket>] attach -t =<session>` in a PTY. `socket` matches
    /// `^[A-Za-z0-9_-]{1,64}$` or the TUI refuses it.
    #[serde(rename_all = "camelCase")]
    Tmux {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        socket: Option<String>,
    },
    /// The session runs in a host process registered as `hosts/<id>.json` in
    /// the garage dir (user-private); the TUI reads address and token there.
    /// The token is never sent over HTTP. An empty `id` means "no such host".
    #[serde(rename_all = "camelCase")]
    Host { id: String },
}

/// The tmux socket-name rule shared by the daemon (what it may advertise) and
/// the TUI (what it will pass to `tmux -L`).
pub fn is_valid_tmux_socket(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    #[test]
    fn error_codes_are_unique_snake_case_names() {
        let mut seen = std::collections::HashSet::new();
        for c in super::ErrorCode::ALL {
            let name = c.as_str();
            assert!(
                !name.is_empty()
                    && name
                        .split('_')
                        .all(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase())),
                "{name} is not snake_case"
            );
            assert!(seen.insert(name), "{name} appears twice");
        }
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn attach_spec_wire_shape() {
        roundtrip::<AttachSpec>(json!({"kind": "tmux"}));
        roundtrip::<AttachSpec>(json!({"kind": "tmux", "socket": "t1"}));
        roundtrip::<AttachSpec>(json!({"kind": "host", "id": "garage/ws/a"}));
    }

    #[test]
    fn tmux_socket_names_are_a_strict_charset() {
        for ok in ["t1", "gt123-0-abc", "A_b-9", &"a".repeat(64)] {
            assert!(is_valid_tmux_socket(ok), "{ok}");
        }
        for bad in [
            "",
            "a b",
            "../x",
            "/tmp/s",
            "a;rm",
            "é",
            "a\nb",
            &"a".repeat(65),
        ] {
            assert!(!is_valid_tmux_socket(bad), "{bad:?}");
        }
    }

    /// Round-trip a JSON literal through `T` and require the exact same JSON
    /// back: proves both field naming and which optionals are omitted vs null.
    fn roundtrip<T: Serialize + for<'de> Deserialize<'de>>(v: Value) {
        let parsed: T = serde_json::from_value(v.clone()).expect("deserialize");
        assert_eq!(serde_json::to_value(&parsed).unwrap(), v);
    }

    #[test]
    fn live_session_roundtrips() {
        roundtrip::<Session>(json!({
            "id": "garage/ws/a", "workspace": "ws", "label": "a",
            "dir": "/tmp/ws", "attached": true, "createdAt": 1_700_000_000_000_i64,
            "status": "needs-input", "message": "Claude needs your permission to use Bash",
            "since": 1_700_000_005_000_i64, "branch": "main", "title": null,
            "context": {"usedPercentage": 42.5, "source": "statusline"}
        }));
    }

    #[test]
    fn restorable_session_has_no_created_at() {
        let v = json!({
            "id": "garage/ws/b", "workspace": "ws", "label": "b", "dir": "/tmp/ws",
            "attached": false, "status": "restorable", "since": null, "message": null,
            "title": null, "restorable": true, "branch": "@abc1234", "context": null
        });
        roundtrip::<Session>(v.clone());
        let s: Session = serde_json::from_value(v).unwrap();
        assert_eq!(s.status, SessionStatus::Restorable);
        assert_eq!(s.created_at, None);
    }

    #[test]
    fn unknown_status_is_rejected() {
        assert!(serde_json::from_value::<SessionStatus>(json!("busy")).is_err());
    }

    #[test]
    fn delete_meta_response_keeps_meta_flag_only_when_set() {
        roundtrip::<DeleteSessionResponse>(
            json!({"deleted": true, "meta": true, "worktree": null}),
        );
        roundtrip::<DeleteSessionResponse>(json!({
            "deleted": true,
            "worktree": {"path": "/g/worktrees/ws/fix", "branch": "garage/fix",
                         "repoDir": "/r", "target": "main"}
        }));
    }

    #[test]
    fn restart_request_accepts_partial_bodies() {
        let t: SessionTarget = serde_json::from_value(json!({"all": true})).unwrap();
        assert_eq!((t.id, t.all, t.force), (None, Some(true), None));
        roundtrip::<SessionTarget>(json!({"id": "garage/ws/a", "force": true}));
    }

    #[test]
    fn rename_workspace_failed_sessions_omitted_when_none() {
        roundtrip::<RenameWorkspaceResponse>(json!({
            "name": "new", "dir": "/d",
            "renamedSessions": [{"from": "garage/old/a", "to": "garage/new/a"}]
        }));
    }

    #[test]
    fn usage_accepts_null_windows_and_null_reset() {
        roundtrip::<Usage>(json!({
            "fiveHour": {"usedPercentage": 42.0, "resetsAt": "2026-10-31T12:00:00Z"},
            "sevenDay": null
        }));
        roundtrip::<UsageWindow>(json!({"usedPercentage": 1.0, "resetsAt": null}));
    }

    #[test]
    fn statusline_snippet_uses_status_line_type_key() {
        roundtrip::<StatuslineSnippet>(json!({
            "statusLine": {"type": "command", "command": "/h/.garage/statusline.sh"},
            "script": "#!/bin/sh\n"
        }));
    }

    #[test]
    fn sse_frames_match_events_js() {
        let e = Event::Status(StatusEvent {
            id: "garage/ws/a".into(),
            status: SessionStatus::NeedsInput,
            since: Some(1700),
        });
        assert_eq!(
            e.frame(),
            "event: status\ndata: {\"id\":\"garage/ws/a\",\"status\":\"needs-input\",\"since\":1700}\n\n"
        );
        assert_eq!(Event::Sessions.frame(), "event: sessions\ndata: {}\n\n");
    }
}
