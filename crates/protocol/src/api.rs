//! The plugin API v1 as types, and its JSON schema (`schemars`).
//!
//! The widget and plugin-list types live in [`crate::widgets`] and
//! [`crate::plugins`] and are what the daemon and the wall really use. This
//! module adds the shapes the daemon used to build with `json!`: request
//! bodies, small replies, the run context, event payloads and the features
//! document. They are checked against the real daemon output by tests, so the
//! committed schema (`schemas/garage-plugin-api.schema.json`) and the SDKs
//! built from it cannot drift from the code.
//!
//! Field names here are `snake_case` (the plugin-facing API), except the core
//! types re-exported from [`crate`] (`Session`, `Workspace`, `Usage`), which
//! are `camelCase` as the core API is.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plugins::{PanesResponse, PluginList, UiActionFrame, UiTarget, WidgetsSnapshot};
use crate::widgets::{WidgetItem, WidgetPut};
use crate::{Ack, ErrorBody, Health, Session, Usage, Workspace};

/// A property that is always present but may be `null`: required, and the
/// type allows `null` (schemars drops `null` from a plain `required` option).
fn nullable<T: JsonSchema>(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
    Option::<T>::json_schema(g)
}

// ---- requests and small replies ----

/// `POST /api/plugins/ui/action` (needs `ui.invoke`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UiActionRequest {
    /// A core id, `plugin:action`, or a bare local action id.
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(default)]
    pub target: UiTarget,
}

/// `POST /api/plugins/{id}/notice` (needs `ui.notice`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoticeRequest {
    /// Cut to 120 characters.
    pub text: String,
    /// 1000..=10000, default 5000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u32>,
}

/// Refuse the input (409) when the session is in one of these statuses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IfStatus {
    pub not: Vec<String>,
}

/// `POST /api/sessions/input` (needs `sessions.input`, dangerous).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionInputRequest {
    /// A `garage/<workspace>/<label>` session id.
    pub id: String,
    /// Typed into the session, then Enter.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_status: Option<IfStatus>,
}

/// Reply of `PUT /api/plugins/{id}/widgets/{widget}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PutWidgetResponse {
    pub ok: bool,
    /// Grows on every change of the stored content.
    pub rev: u64,
}

/// Reply of `POST /api/plugins/{id}/panes/{pane}/open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpenPaneResponse {
    pub ok: bool,
    /// The backend session id of the pane.
    pub session: String,
    /// The pane was already open (a singleton) and was reused.
    pub reused: bool,
}

/// Reply of `DELETE /api/plugins/{id}/panes/{pane}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClosePaneResponse {
    pub ok: bool,
    pub closed: bool,
}

// ---- the run context ----

/// The JSON every action, event, pane and CLI run receives: in
/// `GARAGE_CONTEXT` (or the file `GARAGE_CONTEXT_FILE` names) and on stdin.
/// Keys are never absent; a missing value is `null`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunContext {
    pub api_version: u32,
    pub plugin: ContextPlugin,
    pub trigger: ContextTrigger,
    pub focus: ContextFocus,
    pub selection: ContextSelection,
    /// The focused session's worktree, if it has one.
    #[schemars(required, schema_with = "nullable::<ContextWorktree>")]
    pub worktree: Option<ContextWorktree>,
    /// Set for event hook runs.
    #[schemars(required, schema_with = "nullable::<ContextEvent>")]
    pub event: Option<ContextEvent>,
    pub ui: ContextUi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContextPlugin {
    pub id: String,
    pub version: String,
    /// The plugin's own writable folder.
    pub state_dir: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContextTrigger {
    /// `key`, `button`, `widget`, `palette`, `event`, `timer`, `pane` or `cli`.
    pub kind: String,
    #[schemars(required, schema_with = "nullable::<String>")]
    pub key: Option<String>,
    #[schemars(required, schema_with = "nullable::<String>")]
    pub slot: Option<String>,
    #[schemars(required, schema_with = "nullable::<String>")]
    pub action: Option<String>,
    /// The `args` of the button or click; `{}` when none. For a `cli` run,
    /// `{"argv": [...]}`.
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContextFocus {
    #[schemars(required, schema_with = "nullable::<String>")]
    pub workspace: Option<String>,
    #[schemars(required, schema_with = "nullable::<ContextSession>")]
    pub session: Option<ContextSession>,
    /// CLI runs only: the caller's working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContextSession {
    pub id: String,
    #[schemars(required, schema_with = "nullable::<String>")]
    pub dir: Option<String>,
    #[schemars(required, schema_with = "nullable::<String>")]
    pub status: Option<String>,
    #[schemars(required, schema_with = "nullable::<String>")]
    pub branch: Option<String>,
    #[schemars(required, schema_with = "nullable::<ContextWorktree>")]
    pub worktree: Option<ContextWorktree>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContextWorktree {
    pub path: String,
    pub branch: String,
    pub repo_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContextSelection {
    #[schemars(required, schema_with = "nullable::<String>")]
    pub session: Option<String>,
    #[schemars(required, schema_with = "nullable::<u64>")]
    pub index: Option<u64>,
    #[schemars(required, schema_with = "nullable::<String>")]
    pub view: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContextEvent {
    /// One of the event names (see [`EventName`]).
    pub name: String,
    /// Unix milliseconds.
    pub at: i64,
    /// The payload; its shape depends on `name` (see [`EventPayloads`]).
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContextUi {
    /// Slot geometry the wall reported (`width`, `max_cols`, `filler`), by
    /// slot name. May be empty.
    pub slots: Value,
}

// ---- events ----

/// Event names a manifest `[[events]] on` may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum EventName {
    #[serde(rename = "session.created")]
    SessionCreated,
    #[serde(rename = "session.closed")]
    SessionClosed,
    #[serde(rename = "session.turn")]
    SessionTurn,
    #[serde(rename = "session.forgotten")]
    SessionForgotten,
    #[serde(rename = "session.status_changed")]
    SessionStatusChanged,
    #[serde(rename = "worktree.created")]
    WorktreeCreated,
    #[serde(rename = "workspace.added")]
    WorkspaceAdded,
    #[serde(rename = "workspace.removed")]
    WorkspaceRemoved,
    #[serde(rename = "daemon.started")]
    DaemonStarted,
    #[serde(rename = "timer")]
    Timer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionCreated {
    pub id: String,
    pub dir: String,
    pub workspace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionClosed {
    pub id: String,
    pub dir: String,
    pub workspace: String,
    /// `user`, `crash` or `daemon_restart`.
    pub reason: String,
    #[schemars(required, schema_with = "nullable::<String>")]
    pub last_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionTurn {
    pub id: String,
    pub workspace: String,
    /// Unix milliseconds.
    pub at: i64,
    #[schemars(required, schema_with = "nullable::<u64>")]
    pub transcript_offset: Option<u64>,
    /// Absolute scrollback row where the turn ended (0 is the oldest kept line).
    #[schemars(required, schema_with = "nullable::<u64>")]
    pub row: Option<u64>,
    #[schemars(required, schema_with = "nullable::<u64>")]
    pub history_limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionForgotten {
    pub id: String,
    pub workspace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionStatusChanged {
    pub id: String,
    pub from: crate::SessionStatus,
    pub to: crate::SessionStatus,
    #[schemars(required, schema_with = "nullable::<i64>")]
    pub since: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorktreeCreated {
    pub workspace: String,
    pub session: String,
    pub path: String,
    pub branch: String,
    pub repo_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceNamed {
    pub name: String,
}

/// The payload (`event.data`) of each event, by event name. `daemon.started`
/// and `timer` carry `{}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventPayloads {
    #[serde(rename = "session.created")]
    pub session_created: SessionCreated,
    #[serde(rename = "session.closed")]
    pub session_closed: SessionClosed,
    #[serde(rename = "session.turn")]
    pub session_turn: SessionTurn,
    #[serde(rename = "session.forgotten")]
    pub session_forgotten: SessionForgotten,
    #[serde(rename = "session.status_changed")]
    pub session_status_changed: SessionStatusChanged,
    #[serde(rename = "worktree.created")]
    pub worktree_created: WorktreeCreated,
    #[serde(rename = "workspace.added")]
    pub workspace_added: WorkspaceNamed,
    #[serde(rename = "workspace.removed")]
    pub workspace_removed: WorkspaceNamed,
}

// ---- features ----

/// One entry of `features.routes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteInfo {
    pub method: String,
    pub path: String,
    /// `none` or a permission name.
    pub needs: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FeatureLimits {
    pub widget_items: u32,
    pub widget_segments: u32,
    pub widget_cols: u32,
    pub widget_body_bytes: u32,
    pub widget_puts_per_sec: u32,
    pub widget_ttl_secs: u32,
    pub widget_frames: u32,
    pub notice_chars: u32,
}

/// The `features` object. New keys may appear in later garage versions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Features {
    #[serde(rename = "api.v1.frozen")]
    pub api_v1_frozen: bool,
    pub events: Vec<String>,
    pub permissions: Vec<String>,
    pub dangerous_permissions: Vec<String>,
    pub theme_tokens: Vec<String>,
    pub routes: Vec<RouteInfo>,
    pub slots: Vec<String>,
    #[serde(rename = "widgets.click_args")]
    pub widgets_click_args: bool,
    #[serde(rename = "widgets.filler")]
    pub widgets_filler: bool,
    #[serde(rename = "widgets.animated")]
    pub widgets_animated: bool,
    #[serde(rename = "widgets.frame_x")]
    pub widgets_frame_x: bool,
    #[serde(rename = "widgets.slot_geometry")]
    pub widgets_slot_geometry: bool,
    #[serde(rename = "ui.notice")]
    pub ui_notice: bool,
    #[serde(rename = "cli.fallback")]
    pub cli_fallback: bool,
    #[serde(rename = "panes.placements")]
    pub panes_placements: Vec<String>,
    #[serde(rename = "sessions.input")]
    pub sessions_input: bool,
    #[serde(rename = "actions.core_invocable")]
    pub actions_core_invocable: Vec<String>,
    pub limits: FeatureLimits,
}

/// `GET /api/plugin/features`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FeaturesResponse {
    pub api_versions: Vec<u32>,
    /// The garage version.
    pub garage: String,
    pub features: Features,
}

// ---- the schema ----

/// Root of the published schema: one field per public type, so every type is
/// a named definition (the SDK generators turn definitions into types).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApiTypes {
    pub widget_put: WidgetPut,
    pub widget_item: WidgetItem,
    pub put_widget_response: PutWidgetResponse,
    pub ui_action_request: UiActionRequest,
    pub ui_action_frame: UiActionFrame,
    pub notice_request: NoticeRequest,
    pub session_input_request: SessionInputRequest,
    pub open_pane_response: OpenPaneResponse,
    pub close_pane_response: ClosePaneResponse,
    pub plugin_list: PluginList,
    pub widgets_snapshot: WidgetsSnapshot,
    pub panes_response: PanesResponse,
    pub run_context: RunContext,
    pub event_name: EventName,
    pub event_payloads: EventPayloads,
    pub features_response: FeaturesResponse,
    pub health: Health,
    pub sessions: Vec<Session>,
    pub workspaces: Vec<Workspace>,
    pub usage: Usage,
    pub error: ErrorBody,
    pub ack: Ack,
}

/// Pretty draft-07 JSON with a trailing newline, exactly what is committed.
pub fn api_schema_json() -> String {
    let schema = schemars::generate::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<ApiTypes>();
    let mut out = serde_json::to_string_pretty(&schema).expect("schema serialises");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMITTED: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../schemas/garage-plugin-api.schema.json"
    );

    /// The committed schema must match the types. After changing a wire type,
    /// regenerate with `UPDATE_SCHEMA=1 cargo test -p garage-protocol`.
    /// Compared as values, so key order (a build-feature detail) is moot.
    #[test]
    fn the_committed_api_schema_is_not_stale() {
        let fresh = api_schema_json();
        if std::env::var_os("UPDATE_SCHEMA").is_some() {
            std::fs::write(COMMITTED, &fresh).unwrap();
        }
        let committed = std::fs::read_to_string(COMMITTED).expect(
            "schemas/garage-plugin-api.schema.json is missing; run `UPDATE_SCHEMA=1 cargo test -p garage-protocol`",
        );
        let a: Value = serde_json::from_str(&committed).unwrap();
        let b: Value = serde_json::from_str(&fresh).unwrap();
        assert!(
            a == b,
            "schemas/garage-plugin-api.schema.json is stale; run `UPDATE_SCHEMA=1 cargo test -p garage-protocol` and commit it"
        );
    }

    #[test]
    fn the_schema_keeps_the_frozen_wire_names() {
        let schema: Value = serde_json::from_str(&api_schema_json()).unwrap();
        let defs = &schema["definitions"];
        // widget wire format (golden: widget_item_fields, segment_fields, style_fields)
        for (ty, fields) in [
            (
                "WidgetItem",
                &[
                    "priority",
                    "placement",
                    "segments",
                    "frames",
                    "frame_ms",
                    "frame_x",
                    "row",
                    "kind",
                    "row_style",
                    "click",
                    "session",
                ][..],
            ),
            ("Segment", &["text", "style", "click"][..]),
            (
                "Style",
                &["fg", "bg", "bold", "italic", "underline", "dim"][..],
            ),
            ("WidgetPut", &["scopes", "ttl_secs"][..]),
            (
                "RunContext",
                &[
                    "api_version",
                    "plugin",
                    "trigger",
                    "focus",
                    "selection",
                    "worktree",
                    "event",
                    "ui",
                ][..],
            ),
            ("NoticeRequest", &["text", "ttl_ms"][..]),
            ("SessionInputRequest", &["id", "text", "if_status"][..]),
            ("UiActionRequest", &["action", "args", "target"][..]),
        ] {
            let props = defs[ty]["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{ty} has no properties"));
            for f in fields {
                assert!(props.contains_key(*f), "{ty} lost the field {f}");
            }
        }
        // a context key that can be null is still required (never absent)
        let required = defs["RunContext"]["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "event"));
    }
}
