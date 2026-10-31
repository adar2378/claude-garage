//! Closed-session records (p24): when a session leaves the backend the core
//! keeps a bounded record per workspace so a plugin can offer it again
//! (`GET /api/workspaces/{name}/closed`). A session that comes back (restored,
//! or created under the same id) drops its record.

use tokio::task::JoinHandle;

use crate::bus::{DaemonEvent, EventBus};
use crate::git::resolve_branch;
use crate::state::registry::{ClosedRecord, Registry};
use crate::state::status::now_ms;

pub fn spawn(bus: &EventBus, registry: Registry) -> JoinHandle<()> {
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(DaemonEvent::SessionClosed { id, dir, workspace, reason, last_status }) => {
                    let branch = resolve_branch(&dir).await;
                    let rec = ClosedRecord {
                        id,
                        workspace,
                        dir,
                        branch,
                        closed_at: now_ms(),
                        reason: reason.as_str().to_owned(),
                        last_status,
                    };
                    let _ = registry.record_closed(rec).await;
                }
                Ok(DaemonEvent::SessionCreated { id, .. }) => {
                    let _ = registry.remove_closed(&id).await;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}
