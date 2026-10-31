//! The daemon event bus (design.md decision 6): one broadcast channel that the
//! status store, poller, restart route and (later) the tmux control-mode
//! reader publish to, and SSE clients, notifications and plugins subscribe to.
//! Replaces `statusEvents` and `pollerEvents` in the JS daemon.

use garage_protocol::SessionStatus;
use tokio::sync::broadcast;

use crate::closing::CloseReason;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonEvent {
    /// A real status transition (`since` only changes on these).
    StatusTransition {
        id: String,
        from: SessionStatus,
        to: SessionStatus,
        since: Option<i64>,
    },
    /// The set of live sessions or a pane title changed.
    SessionsChanged,
    /// A session appeared (typed counterpart of `SessionsChanged`).
    SessionCreated {
        id: String,
        dir: String,
        workspace: String,
    },
    /// A session left the backend. `last_status` is what the status store
    /// held just before it was dropped.
    SessionClosed {
        id: String,
        dir: String,
        workspace: String,
        reason: CloseReason,
        last_status: Option<String>,
    },
    /// One Claude turn ended (the Stop hook). `transcript_offset` is the
    /// transcript's byte length at that moment when the hook named one.
    SessionTurn {
        id: String,
        workspace: String,
        at: i64,
        transcript_offset: Option<u64>,
    },
    WorktreeCreated {
        workspace: String,
        session: String,
        path: String,
        branch: String,
        repo_dir: String,
    },
    /// A closed session was forgotten (`DELETE /api/sessions/{id}?meta=1`):
    /// its closed record and resume metadata are gone.
    SessionForgotten {
        id: String,
        workspace: String,
    },
    WorkspaceAdded {
        name: String,
    },
    WorkspaceRemoved {
        name: String,
    },
    /// A plugin action or event run failed (non-zero exit, timeout, spawn).
    PluginRunFailed {
        plugin: String,
        action: String,
        reason: String,
    },
    /// A message for the user about a plugin (toast).
    PluginNotice {
        plugin: String,
        message: String,
    },
    /// A plugin's widget content changed; `rev` is its new revision.
    WidgetsChanged {
        plugin: String,
        rev: u64,
    },
    /// The installed set, enabled state or bindings changed.
    PluginsChanged,
    /// A plugin asked the TUI to run an action (`POST /api/plugins/ui/action`).
    /// There is no `confirmed` here on purpose: the TUI alone decides whether
    /// a dangerous action runs.
    UiAction {
        plugin: String,
        action: String,
        args: Option<serde_json::Value>,
        target: garage_protocol::plugins::UiTarget,
    },
    /// A plugin pane opened (`open: true`) or closed. `session` is the
    /// backend session id the TUI attaches to.
    PaneChanged {
        plugin: String,
        pane: String,
        session: String,
        open: bool,
    },
}

impl DaemonEvent {
    /// The plugin-visible name and payload of a bus event, for event hooks
    /// and the typed SSE frames. Internal plugin-host events return `None`.
    pub fn plugin_event(&self) -> Option<(&'static str, serde_json::Value)> {
        use serde_json::json;
        Some(match self {
            DaemonEvent::StatusTransition {
                id,
                from,
                to,
                since,
            } => (
                "session.status_changed",
                json!({"id": id, "from": from, "to": to, "since": since}),
            ),
            DaemonEvent::SessionCreated { id, dir, workspace } => (
                "session.created",
                json!({"id": id, "dir": dir, "workspace": workspace}),
            ),
            DaemonEvent::SessionClosed {
                id,
                dir,
                workspace,
                reason,
                last_status,
            } => (
                "session.closed",
                json!({"id": id, "dir": dir, "workspace": workspace, "reason": reason.as_str(), "last_status": last_status}),
            ),
            DaemonEvent::SessionTurn {
                id,
                workspace,
                at,
                transcript_offset,
            } => (
                "session.turn",
                json!({"id": id, "workspace": workspace, "at": at, "transcript_offset": transcript_offset}),
            ),
            DaemonEvent::SessionForgotten { id, workspace } => (
                "session.forgotten",
                json!({"id": id, "workspace": workspace}),
            ),
            DaemonEvent::WorktreeCreated {
                workspace,
                session,
                path,
                branch,
                repo_dir,
            } => (
                "worktree.created",
                json!({"workspace": workspace, "session": session, "path": path, "branch": branch, "repo_dir": repo_dir}),
            ),
            DaemonEvent::WorkspaceAdded { name } => ("workspace.added", json!({"name": name})),
            DaemonEvent::WorkspaceRemoved { name } => ("workspace.removed", json!({"name": name})),
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<DaemonEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        Self {
            tx: broadcast::channel(capacity).0,
        }
    }

    /// Publish to all current subscribers; having none is not an error.
    pub fn publish(&self, event: DaemonEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(256)
    }
}
