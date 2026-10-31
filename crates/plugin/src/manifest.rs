//! `garage-plugin.toml` as Rust types. All keys are `snake_case` and unknown
//! keys are an error, so a typo surfaces at install. The JSON schema is
//! derived from these types (`crate::schema`).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Macos,
    Linux,
    Windows,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Macos => "macos",
            Platform::Linux => "linux",
            Platform::Windows => "windows",
        }
    }
}

/// What must be focused for an action to be available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ActionContext {
    Global,
    Workspace,
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Permission {
    #[serde(rename = "sessions.read")]
    SessionsRead,
    #[serde(rename = "sessions.input")]
    SessionsInput,
    #[serde(rename = "sessions.manage")]
    SessionsManage,
    #[serde(rename = "workspaces.read")]
    WorkspacesRead,
    #[serde(rename = "workspaces.write")]
    WorkspacesWrite,
    #[serde(rename = "ui.widgets")]
    UiWidgets,
    #[serde(rename = "ui.panes")]
    UiPanes,
    #[serde(rename = "ui.invoke")]
    UiInvoke,
    #[serde(rename = "core.actions")]
    CoreActions,
    // Navigation only: move focus or view (never a close, delete or restart).
    #[serde(rename = "ui.navigate")]
    UiNavigate,
    // Show a short notice (toast) in the TUI.
    #[serde(rename = "ui.notice")]
    UiNotice,
    #[serde(rename = "network")]
    Network,
}

impl Permission {
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::SessionsRead => "sessions.read",
            Permission::SessionsInput => "sessions.input",
            Permission::SessionsManage => "sessions.manage",
            Permission::WorkspacesRead => "workspaces.read",
            Permission::WorkspacesWrite => "workspaces.write",
            Permission::UiWidgets => "ui.widgets",
            Permission::UiPanes => "ui.panes",
            Permission::UiInvoke => "ui.invoke",
            Permission::CoreActions => "core.actions",
            Permission::UiNavigate => "ui.navigate",
            Permission::UiNotice => "ui.notice",
            Permission::Network => "network",
        }
    }

    pub fn is_dangerous(self) -> bool {
        crate::features::is_dangerous(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
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

impl EventName {
    pub fn as_str(self) -> &'static str {
        match self {
            EventName::SessionCreated => "session.created",
            EventName::SessionClosed => "session.closed",
            EventName::SessionTurn => "session.turn",
            EventName::SessionForgotten => "session.forgotten",
            EventName::SessionStatusChanged => "session.status_changed",
            EventName::WorktreeCreated => "worktree.created",
            EventName::WorkspaceAdded => "workspace.added",
            EventName::WorkspaceRemoved => "workspace.removed",
            EventName::DaemonStarted => "daemon.started",
            EventName::Timer => "timer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum Slot {
    #[serde(rename = "tile.title.right")]
    TileTitleRight,
    #[serde(rename = "tile.gutter")]
    TileGutter,
    #[serde(rename = "rail.row")]
    RailRow,
    #[serde(rename = "rail.extra_rows")]
    RailExtraRows,
    #[serde(rename = "rail.header")]
    RailHeader,
    #[serde(rename = "strip.right")]
    StripRight,
}

impl Slot {
    pub fn to_protocol(self) -> garage_protocol::widgets::WidgetSlot {
        use garage_protocol::widgets::WidgetSlot as W;
        match self {
            Slot::TileTitleRight => W::TileTitleRight,
            Slot::TileGutter => W::TileGutter,
            Slot::RailRow => W::RailRow,
            Slot::RailExtraRows => W::RailExtraRows,
            Slot::RailHeader => W::RailHeader,
            Slot::StripRight => W::StripRight,
        }
    }

    pub fn as_str(self) -> &'static str {
        self.to_protocol().as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PanePlacement {
    Tile,
    Overlay,
    Popup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Unique id: `^[a-z][a-z0-9-]{1,30}$`, not a reserved word.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Semver.
    pub version: String,
    /// The plugin API major version.
    pub api_version: u32,
    /// Systems the plugin runs on.
    pub platforms: Vec<Platform>,
    /// One line for the install preview and `plugin list`.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    /// A URL shown in the install preview.
    pub homepage: Option<String>,
    /// Feature names from `/api/plugin/features` the plugin needs.
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    /// What the plugin uses. Shown at install; the daemon checks it on API calls.
    pub permissions: Vec<Permission>,
    /// Commands bindable to keys, buttons and the palette (`[[actions]]`).
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Commands run when something happens (`[[events]]`).
    #[serde(default)]
    pub events: Vec<EventHook>,
    /// Programs shown in a tile, overlay or popup (`[[panes]]`).
    #[serde(default)]
    pub panes: Vec<Pane>,
    /// Named spots the plugin fills at runtime (`[[widgets]]`).
    #[serde(default)]
    pub widgets: Vec<Widget>,
    /// Clickable labels that run an action (`[[buttons]]`).
    #[serde(default)]
    pub buttons: Vec<Button>,
    /// Subcommands of `claude-garage` (`[[cli]]`).
    #[serde(default)]
    pub cli: Vec<Cli>,
    /// Commands run once at install and update (`[[build]]`).
    #[serde(default)]
    pub build: Vec<Build>,
    /// Per-system binary downloads that replace `[[build]]` (`[release]`).
    #[serde(default)]
    pub release: Option<Release>,
}

fn yes() -> bool {
    true
}

/// Bindable to keys and buttons. Exactly one of `command` / `pane`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Action {
    /// Local id; the full id is `<plugin>:<id>`.
    pub id: String,
    /// Shown in help and the palette.
    pub title: String,
    /// What must be focused for the action to be available.
    #[serde(default = "default_contexts")]
    pub contexts: Vec<ActionContext>,
    /// The program and arguments to run (no shell). Exactly one of `command` and `pane`.
    #[serde(default)]
    pub command: Option<Vec<String>>,
    /// Open one of this plugin's panes instead of running a command.
    #[serde(default)]
    pub pane: Option<String>,
    /// A suggestion; never overrides an existing binding.
    #[serde(default)]
    pub default_key: Option<String>,
    /// Ask the user before a plugin-origin trigger (click, `ui/action`) runs it.
    #[serde(default)]
    pub confirm: bool,
    /// 1..=300, default 30.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
}

fn default_contexts() -> Vec<ActionContext> {
    vec![ActionContext::Global]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventHook {
    /// The event, or `timer`.
    pub on: EventName,
    /// Only with `on = "timer"`; at least 5.
    #[serde(default)]
    pub every_secs: Option<u32>,
    /// The program and arguments to run (no shell).
    pub command: Vec<String>,
    /// 1..=60, default 10.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaneSize {
    /// Columns (or percent of the screen width).
    pub cols: u16,
    /// Rows (or percent of the screen height).
    pub rows: u16,
    /// `cols`/`rows` are percent of the screen.
    #[serde(default)]
    pub pct: bool,
}

/// Runs as a backend session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pane {
    /// Local id, used in `pane = "..."` and the open-pane route.
    pub id: String,
    /// Shown in the pane frame.
    pub title: String,
    /// Where it appears.
    pub placement: PanePlacement,
    /// The program and arguments to run (no shell).
    pub command: Vec<String>,
    /// Opening again focuses the existing pane. Default true.
    #[serde(default = "yes")]
    pub singleton: bool,
    #[serde(default)]
    /// A size hint for overlay and popup panes.
    pub size: Option<PaneSize>,
}

/// Content is pushed at runtime over the daemon API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Widget {
    /// Local id, used in the push route.
    pub id: String,
    /// Where it is drawn; decides the scope key of its content.
    pub slot: Slot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Button {
    /// Where the button is drawn.
    pub slot: Slot,
    /// The text of the button.
    pub label: String,
    #[serde(default)]
    /// Shown on hover.
    pub tooltip: Option<String>,
    /// A core id, `plugin:action`, or a bare local action id.
    pub on_click: String,
    /// JSON passed as `trigger.args`.
    #[serde(default)]
    pub args: Option<Value>,
    /// May only add confirmation; a core action's own cannot be removed.
    #[serde(default)]
    pub confirm: bool,
    /// Higher survives narrow widths.
    #[serde(default)]
    pub priority: i32,
}

/// Exposed as `claude-garage <name>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Cli {
    /// The subcommand: `claude-garage <name>`.
    pub name: String,
    /// Shown in help.
    #[serde(default)]
    pub description: Option<String>,
    /// The program and arguments to run (no shell).
    pub command: Vec<String>,
    /// Also receives an unknown first word. At most one installed plugin.
    #[serde(default)]
    pub fallback: bool,
    /// `claude-garage` makes sure a current daemon is running before the
    /// command starts (the command calls the daemon API).
    #[serde(default)]
    pub needs_daemon: bool,
}

/// Run once at install.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Build {
    /// Default: all header platforms.
    #[serde(default)]
    pub platforms: Vec<Platform>,
    /// The program and arguments to run (no shell).
    pub command: Vec<String>,
    /// Default 600.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
}

/// Per-OS binary assets; when the current target is listed `[[build]]` is
/// skipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// Default: the repo the plugin was installed from.
    #[serde(default)]
    pub repo: Option<String>,
    /// Template; `{version}` is replaced.
    #[serde(default)]
    pub tag: Option<String>,
    /// Asset holding `<sha256>  <asset>` lines.
    pub checksums: String,
    /// `os-arch` (for example `macos-aarch64`) to asset name.
    #[serde(default)]
    pub assets: BTreeMap<String, String>,
}

/// Lowercase hex sha256, for `manifestSha256` in `plugins.json`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub const DEFAULT_ACTION_TIMEOUT_SECS: u32 = 30;
pub const DEFAULT_EVENT_TIMEOUT_SECS: u32 = 10;
pub const DEFAULT_BUILD_TIMEOUT_SECS: u32 = 600;

impl Manifest {
    pub fn has_permission(&self, p: Permission) -> bool {
        self.permissions.contains(&p)
    }

    pub fn permission_names(&self) -> Vec<String> {
        self.permissions
            .iter()
            .map(|p| p.as_str().to_owned())
            .collect()
    }

    pub fn action(&self, id: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.id == id)
    }

    pub fn pane(&self, id: &str) -> Option<&Pane> {
        self.panes.iter().find(|p| p.id == id)
    }

    pub fn widget(&self, id: &str) -> Option<&Widget> {
        self.widgets.iter().find(|w| w.id == id)
    }
}
