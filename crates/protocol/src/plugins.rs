//! What the TUI reads about the plugin host (p24 design section 7): the
//! installed set with the actions, buttons and panes the wall binds, the
//! widget snapshot, the pane list, and the `ui_action` frame. Daemon and wall
//! share these types, so the wall never depends on the manifest crate.
//!
//! Field names are `snake_case` (the plugin-facing API).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AttachSpec;
use crate::widgets::{WidgetItem, WidgetSlot};

/// `GET /api/plugins`: the plugins in install order (which is also the
/// tiebreak for two plugins that suggest the same key).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PluginList {
    pub plugins: Vec<PluginSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Enabled, approved, loaded: only an active plugin's actions, buttons
    /// and widgets reach the screen.
    pub active: bool,
    #[serde(default)]
    pub version: Option<String>,
    /// Where it came from: `bundled`, `local` (a linked folder) or `github`.
    /// Empty from a daemon that does not say.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    /// The user's own key bindings (`action` local id to key).
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub actions: Vec<ActionSummary>,
    #[serde(default)]
    pub buttons: Vec<ButtonSummary>,
    #[serde(default)]
    pub widgets: Vec<WidgetSummary>,
    #[serde(default)]
    pub panes: Vec<PaneSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionSummary {
    /// The full id, `plugin:action`.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub default_key: Option<String>,
    /// `global`, `workspace` or `session`: what must be focused.
    #[serde(default)]
    pub contexts: Vec<String>,
    #[serde(default)]
    pub confirm: bool,
    /// The local pane id when the action opens a pane instead of running a command.
    #[serde(default)]
    pub pane: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ButtonSummary {
    pub slot: WidgetSlot,
    pub label: String,
    #[serde(default)]
    pub tooltip: Option<String>,
    /// A core id, `plugin:action`, or a bare local id.
    pub on_click: String,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WidgetSummary {
    pub id: String,
    pub slot: WidgetSlot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PaneSummary {
    pub id: String,
    pub title: String,
    /// `tile`, `overlay` or `popup`.
    pub placement: String,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
    /// `cols`/`rows` are percent of the screen.
    #[serde(default)]
    pub pct: bool,
}

/// One stored widget as `GET /api/plugins/widgets` returns it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WidgetContent {
    pub slot: WidgetSlot,
    pub scopes: BTreeMap<String, Vec<WidgetItem>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PluginWidgets {
    pub rev: u64,
    pub widgets: BTreeMap<String, WidgetContent>,
}

/// `GET /api/plugins/widgets[?plugin=]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WidgetsSnapshot {
    pub plugins: BTreeMap<String, PluginWidgets>,
}

/// One open plugin pane, with how to attach to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PaneInstance {
    pub plugin: String,
    pub pane: String,
    pub title: String,
    pub placement: String,
    /// The backend session id (`plugin/<plugin>/<pane>`); not a `garage/` id,
    /// so it is never in the session list.
    pub session: String,
    pub attach: AttachSpec,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
    #[serde(default)]
    pub pct: bool,
}

/// `GET /api/plugins/panes`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PanesResponse {
    pub panes: Vec<PaneInstance>,
}

/// Where a `ui_action` request applies. Core actions take their session and
/// workspace from here and nowhere else.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UiTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

/// The SSE `ui_action` frame: a plugin asked the TUI to run an action. There
/// is deliberately no `confirmed` field: whether a dangerous action runs is
/// the human's decision at the TUI, never the plugin's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UiActionFrame {
    pub plugin: String,
    pub action: String,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(default)]
    pub target: UiTarget,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_ui_action_frame_ignores_a_confirmed_field_and_never_writes_one() {
        let frame: UiActionFrame = serde_json::from_value(
            json!({"plugin": "p", "action": "session.close", "confirmed": true, "target": {"session": "garage/w/a", "confirmed": true}}),
        )
        .unwrap();
        assert_eq!(frame.target.session.as_deref(), Some("garage/w/a"));
        let back = serde_json::to_string(&frame).unwrap();
        assert!(!back.contains("confirmed"), "{back}");
    }

    #[test]
    fn a_listing_with_missing_optional_pieces_still_parses() {
        let list: PluginList = serde_json::from_value(
            json!({"plugins": [{"id": "p", "name": "P", "enabled": true, "active": false}]}),
        )
        .unwrap();
        assert!(list.plugins[0].actions.is_empty() && list.plugins[0].bindings.is_empty());
    }
}
