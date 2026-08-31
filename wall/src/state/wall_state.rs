//! Immutable wall state (port of `tui/lib/state/wall_state.dart` — design:
//! "State architecture": one WallState; every render is a pure function of it
//! plus live terminal buffers). Mutation happens only in `store.rs`.

use std::collections::HashMap;

use crate::api::models::{SessionInfo, WorkspaceInfo};
use crate::state::salience::WorkspaceGroup;
use crate::state::views::{self, View, ViewSummary};

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
    /// `D` (spec tui-views "Move to a group"): the view picker.
    ViewPicker,
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
    /// The session's daemon-normalized OSC/pane title (spec tui-wall "Auto-
    /// subtitle in the tile bar"), when it differs meaningfully from an
    /// empty/hostname/shell name — `None` renders byte-identical to pre-p10.
    pub title: Option<String>,
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
            title: info.title.clone(),
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
    /// Named (non-default) views per workspace, in creation order (spec
    /// tui-views; see `state::views` module docs for the exception-list
    /// model). A session id absent from every list here belongs to the
    /// derived default view — most workspaces never appear in this map at
    /// all. Mutated only by [`crate::state::store::WallStore::detach_focused`]
    /// and pruned on every reconciliation
    /// ([`crate::state::store::WallStore::sessions_fetched`] and friends) —
    /// see [`views::prune_views`].
    pub views: HashMap<String, Vec<View>>,
    /// The focused view name per workspace; absent = [`views::DEFAULT_VIEW`].
    /// Persists across workspace switches within a run (so returning to a
    /// workspace lands back on whichever view you left it on) but is never
    /// itself written to `wall.json` — only membership persists across a
    /// restart (spec tui-views "View persistence" schema has no focus
    /// field).
    pub focused_view: HashMap<String, String>,
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
            views: HashMap::new(),
            focused_view: HashMap::new(),
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

    /// The workspace's views in display order for the view strip and group
    /// frame (spec tui-views "View strip and group frame"): the default
    /// view first if non-empty, then named views in creation order; a view
    /// with no live members doesn't appear (mirrors `views.js`
    /// `computeViews` — see `state::views`). A strip renders only when this
    /// has 2+ entries; a frame renders around the focused view's tiles only
    /// when its `session_ids.len() >= 2`. Empty when `workspace` has no
    /// group at all.
    pub fn views_for(&self, workspace: &str) -> Vec<ViewSummary> {
        let Some(group) = self.group_by_name(Some(workspace)) else {
            return Vec::new();
        };
        let named = self.views.get(workspace).map(Vec::as_slice).unwrap_or(&[]);
        views::compute_views(&group.sessions, named)
    }

    /// `workspace`'s currently focused view name — never a dangling one:
    /// reconciliation resets a workspace's entry to the default the moment
    /// pruning would otherwise leave it pointing at a view that no longer
    /// exists (see `store::reconcile`).
    pub fn focused_view_name(&self, workspace: &str) -> &str {
        self.focused_view.get(workspace).map(String::as_str).unwrap_or(views::DEFAULT_VIEW)
    }

    /// Salience-ordered session ids of `workspace`'s `view` — the input to
    /// the per-view grid cap/LRU machinery in store.rs. Empty when the
    /// workspace has no group, or when `view` currently has no members.
    pub(crate) fn view_session_ids(&self, workspace: &str, view: &str) -> Vec<String> {
        let Some(group) = self.group_by_name(Some(workspace)) else {
            return Vec::new();
        };
        let named = self.views.get(workspace).map(Vec::as_slice).unwrap_or(&[]);
        group
            .sessions
            .iter()
            .filter(|s| views::view_of(named, &s.id) == view)
            .map(|s| s.id.clone())
            .collect()
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
                Some(OverlayKind::ViewPicker) => "keys → move to group".to_owned(),
                _ => "keys → help".to_owned(),
            },
        }
    }
}
