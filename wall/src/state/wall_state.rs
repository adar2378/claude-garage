//! Immutable wall state (port of `tui/lib/state/wall_state.dart` — design:
//! "State architecture": one WallState; every render is a pure function of it
//! plus live terminal buffers). Mutation happens only in `store.rs`.

use crate::api::models::{SessionInfo, WorkspaceInfo};
use crate::state::salience::WorkspaceGroup;

/// Exactly one layer governs keyboard input at a time
/// (spec tui-key-routing: "Three key layers with a visible target chip").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyLayer {
    Garage,
    Engaged,
    Overlay,
}

/// Which overlay is open while [`KeyLayer::Overlay`] is active. Overlays
/// never stack: at most one is open, tracked by a single slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    Help,
    TriageQueue,
    WorkspaceAdd,
}

/// One session as the wall tracks it. Mirrors the daemon entry plus the
/// derived `worktree` flag (the listing does not expose the worktree record;
/// like the web UI, a session whose starting dir differs from its
/// workspace's registered dir was spawned into a worktree).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WallSession {
    pub id: String,
    pub workspace: String,
    pub label: String,
    pub dir: Option<String>,
    /// `needs-input | working | done | idle | restorable`.
    pub status: String,
    /// Epoch ms the current status began.
    pub since: Option<i64>,
    /// Notification text; non-`None` only while needs-input.
    pub message: Option<String>,
    pub branch: Option<String>,
    pub worktree: bool,
}

impl WallSession {
    pub fn from_info(info: &SessionInfo, workspace_dir: Option<&str>) -> WallSession {
        WallSession {
            id: info.id.clone(),
            workspace: info.workspace.clone(),
            label: info.label.clone(),
            dir: info.dir.clone(),
            status: info.status.clone(),
            since: info.since,
            message: info.message.clone(),
            branch: info.branch.clone(),
            worktree: !info.restorable
                && info.dir.is_some()
                && workspace_dir.is_some()
                && info.dir.as_deref() != workspace_dir,
        }
    }

    pub fn needs_input(&self) -> bool {
        self.status == "needs-input"
    }

    /// A live session has a tmux session behind it — anything but restorable.
    pub fn live(&self) -> bool {
        self.status != "restorable"
    }

    /// Port of the Dart `copyWith`: status replaced when given; since/message
    /// are replaced, not merged — a status transition always carries its own
    /// values (`None` clears).
    pub fn copy_with(
        &self,
        status: Option<&str>,
        since: Option<i64>,
        message: Option<String>,
    ) -> WallSession {
        WallSession {
            status: status.map(str::to_owned).unwrap_or_else(|| self.status.clone()),
            since,
            message,
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug)]
pub struct WallState {
    /// Raw registry order, as fetched.
    pub workspaces: Vec<WorkspaceInfo>,
    /// All sessions, as fetched (live + restorable).
    pub sessions: Vec<WallSession>,
    /// Salience-ordered groups (see salience.rs). This IS the rail render
    /// order and the order the `1`–`9` bindings index into.
    pub groups: Vec<WorkspaceGroup>,
    /// Focused workspace by name — name, not index, so salience reorders
    /// never silently move the focus.
    pub focused_workspace: Option<String>,
    pub focused_session_id: Option<String>,
    pub layer: KeyLayer,
    /// Non-`None` exactly while [`WallState::layer`] == [`KeyLayer::Overlay`].
    pub overlay: Option<OverlayKind>,
    /// The tile taking the full grid area (`m` toggle, spec tui-wall
    /// "Maximized tile"). Always one of `gridded_session_ids`; cleared
    /// whenever focus moves to a different session, and by reconciliation
    /// when the session leaves the grid.
    pub maximized_session_id: Option<String>,
    /// Session ids in the grid for the focused workspace, display order,
    /// length <= [`WallState::GRID_CAP`]. Slot-stable: swap-ins replace the
    /// evicted tile's slot instead of reshuffling the grid.
    pub gridded_session_ids: Vec<String>,
    /// Focus recency for LRU eviction — least-recently-focused first.
    pub grid_focus_recency: Vec<String>,
}

impl Default for WallState {
    fn default() -> WallState {
        WallState::initial()
    }
}

impl WallState {
    /// At most this many tiles render at once (spec tui-wall: six-tile cap).
    pub const GRID_CAP: usize = 6;

    pub fn initial() -> WallState {
        WallState {
            workspaces: Vec::new(),
            sessions: Vec::new(),
            groups: Vec::new(),
            focused_workspace: None,
            focused_session_id: None,
            layer: KeyLayer::Garage,
            overlay: None,
            maximized_session_id: None,
            gridded_session_ids: Vec::new(),
            grid_focus_recency: Vec::new(),
        }
    }

    pub fn session_by_id(&self, id: Option<&str>) -> Option<&WallSession> {
        let id = id?;
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn group_by_name(&self, name: Option<&str>) -> Option<&WorkspaceGroup> {
        let name = name?;
        self.groups.iter().find(|g| g.name == name)
    }

    /// Count of needs-input sessions across all workspaces (strip badge).
    pub fn blocked_count(&self) -> usize {
        self.sessions.iter().filter(|s| s.needs_input()).count()
    }

    /// The bottom strip's keys-target chip
    /// (spec: `keys → garage` / `keys → <workspace>/<label>`).
    pub fn keys_target_chip(&self) -> String {
        match self.layer {
            KeyLayer::Garage => "keys → garage".to_owned(),
            KeyLayer::Engaged => match self.session_by_id(self.focused_session_id.as_deref()) {
                None => "keys → garage".to_owned(),
                Some(s) => format!("keys → {}/{}", s.workspace, s.label),
            },
            KeyLayer::Overlay => match self.overlay {
                Some(OverlayKind::TriageQueue) => "keys → queue".to_owned(),
                Some(OverlayKind::WorkspaceAdd) => "keys → add workspace".to_owned(),
                _ => "keys → help".to_owned(),
            },
        }
    }
}
