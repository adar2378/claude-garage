//! The daemon event bus (design.md decision 6): one broadcast channel that the
//! status store, poller, restart route and (later) the tmux control-mode
//! reader publish to, and SSE clients, notifications and plugins subscribe to.
//! Replaces `statusEvents` and `pollerEvents` in the JS daemon.

use garage_protocol::SessionStatus;
use tokio::sync::broadcast;

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
}

#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<DaemonEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        Self { tx: broadcast::channel(capacity).0 }
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
