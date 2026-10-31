//! Closed-session records (p24): when a session leaves the backend the core
//! keeps a bounded record per workspace so a plugin can offer it again
//! (`GET /api/workspaces/{name}/closed`). A session that comes back (restored,
//! or created under the same id) drops its record.
//!
//! A record carries the Claude session id, so restoring it resumes the
//! conversation. The bus is lossy: when this task falls behind it cannot know
//! which closes and creates it missed, so it rescans the backend and the
//! registry and reconciles ([`reconcile`]).

use std::collections::HashSet;
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::backend::SessionBackend;
use crate::bus::{DaemonEvent, EventBus};
use crate::git::resolve_branch;
use crate::state::registry::{ClosedRecord, Registry};
use crate::state::status::now_ms;

pub fn spawn(
    bus: &EventBus,
    registry: Registry,
    backend: Arc<dyn SessionBackend>,
) -> JoinHandle<()> {
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(DaemonEvent::SessionClosed {
                    id,
                    dir,
                    workspace,
                    reason,
                    last_status,
                }) => {
                    // The resume id lives in the session's metadata, which a
                    // deliberate close has already removed; the record the
                    // close path wrote first still has it.
                    let meta = registry.get_session_meta(&id).await;
                    let prior = registry.get_closed(&id).await;
                    let claude_session_id =
                        match meta.as_ref().and_then(|m| m.claude_session_id.clone()) {
                            Some(c) => Some(c),
                            None => prior.as_ref().and_then(|r| r.claude_session_id.clone()),
                        };
                    let worktree = meta
                        .and_then(|m| m.worktree)
                        .or_else(|| prior.and_then(|r| r.worktree));
                    let branch = resolve_branch(&dir).await;
                    let rec = ClosedRecord {
                        id,
                        workspace,
                        dir,
                        branch,
                        closed_at: now_ms(),
                        reason: reason.as_str().to_owned(),
                        last_status,
                        claude_session_id,
                        worktree,
                    };
                    let _ = registry.record_closed(rec).await;
                }
                Ok(DaemonEvent::SessionCreated { id, .. }) => {
                    let _ = registry.remove_closed(&id).await;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    eprintln!(
                        "closed: fell behind the bus and missed {missed} events; reconciling closed records with the backend"
                    );
                    reconcile(&registry, backend.as_ref()).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// Rebuild what lost bus events would have said. A record for a session that
/// is live again goes (its `SessionCreated` was lost). A session with resume
/// metadata that is neither live nor recorded gets a `crash` record (its
/// `SessionClosed` was lost; the real reason is unknown). A failing backend
/// listing changes nothing.
pub async fn reconcile(registry: &Registry, backend: &dyn SessionBackend) {
    let Ok(live) = backend.list().await else {
        return;
    };
    let live: HashSet<String> = live.into_iter().map(|s| s.id).collect();
    let workspaces = registry.list_workspaces().await;
    for ws in &workspaces {
        for rec in registry.list_closed(&ws.name).await {
            if live.contains(&rec.id) {
                let _ = registry.remove_closed(&rec.id).await;
            }
        }
    }
    for (id, meta) in registry.list_session_metas().await {
        if live.contains(&id) || registry.get_closed(&id).await.is_some() {
            continue;
        }
        let (workspace, _) = crate::backend::tmux::split_session_id(&id);
        let dir = match meta.worktree.as_ref().map(|w| w.path.clone()) {
            Some(d) => Some(d),
            None => workspaces
                .iter()
                .find(|w| w.name == workspace)
                .and_then(|w| w.dir.clone()),
        };
        let Some(dir) = dir else { continue };
        let branch = resolve_branch(&dir).await;
        let rec = ClosedRecord {
            id,
            workspace,
            dir,
            branch,
            closed_at: now_ms(),
            reason: "crash".into(),
            last_status: None,
            claude_session_id: meta.claude_session_id,
            worktree: meta.worktree,
        };
        let _ = registry.record_closed(rec).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::closing::CloseReason;
    use crate::poller::fakes::FakeBackend;
    use crate::state::registry::MetaUpdate;

    struct Rig {
        _dir: tempfile::TempDir,
        registry: Registry,
        backend: Arc<FakeBackend>,
        bus: EventBus,
    }

    fn rig(capacity: usize) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        Rig {
            registry: Registry::new(dir.path()),
            _dir: dir,
            backend: Arc::new(FakeBackend::new()),
            bus: EventBus::new(capacity),
        }
    }

    fn closed_event(id: &str) -> DaemonEvent {
        DaemonEvent::SessionClosed {
            id: id.into(),
            dir: "/nonexistent-dir".into(),
            workspace: "ws".into(),
            reason: CloseReason::Crash,
            last_status: None,
        }
    }

    async fn meta(r: &Rig, id: &str, csid: &str) {
        r.registry
            .upsert_session_meta(
                id,
                MetaUpdate {
                    claude_session_id: Some(Some(csid.into())),
                    workspace: Some("ws".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    async fn until<F: std::future::Future<Output = bool>>(f: impl Fn() -> F) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !f().await {
            assert!(std::time::Instant::now() < end, "timed out");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn a_close_event_copies_the_claude_session_id_from_the_session_metadata() {
        let r = rig(64);
        r.registry.upsert_workspace("ws", "/w").await.unwrap();
        meta(&r, "garage/ws/a", "conv-1").await;
        let _t = spawn(&r.bus, r.registry.clone(), r.backend.clone());
        r.bus.publish(closed_event("garage/ws/a"));
        until(|| async { r.registry.get_closed("garage/ws/a").await.is_some() }).await;
        let rec = r.registry.get_closed("garage/ws/a").await.unwrap();
        assert_eq!(
            (rec.claude_session_id.as_deref(), rec.reason.as_str()),
            (Some("conv-1"), "crash")
        );
    }

    #[tokio::test]
    async fn a_session_that_comes_back_drops_its_record() {
        let r = rig(64);
        let _t = spawn(&r.bus, r.registry.clone(), r.backend.clone());
        r.bus.publish(closed_event("garage/ws/a"));
        until(|| async { r.registry.get_closed("garage/ws/a").await.is_some() }).await;
        r.bus.publish(DaemonEvent::SessionCreated {
            id: "garage/ws/a".into(),
            dir: "/d".into(),
            workspace: "ws".into(),
        });
        until(|| async { r.registry.get_closed("garage/ws/a").await.is_none() }).await;
    }

    #[tokio::test]
    async fn after_lagging_it_rescans_instead_of_skipping() {
        // two-slot channel; the task has not run when ten events arrive
        let r = rig(2);
        r.registry.upsert_workspace("ws", "/w").await.unwrap();
        // c1 is still live; its record is stale because SessionCreated was lost
        r.registry
            .record_closed(ClosedRecord {
                id: "garage/ws/back".into(),
                workspace: "ws".into(),
                dir: "/w".into(),
                branch: None,
                closed_at: 1,
                reason: "crash".into(),
                last_status: None,
                claude_session_id: None,
                worktree: None,
            })
            .await
            .unwrap();
        r.backend.set("garage/ws/back", "/w", Some(1), "");
        // lost1 and lost2 vanished while the close events were dropped
        meta(&r, "garage/ws/lost1", "conv-a").await;
        meta(&r, "garage/ws/lost2", "conv-b").await;
        // a live session with metadata must not be recorded
        meta(&r, "garage/ws/back", "conv-c").await;
        let _t = spawn(&r.bus, r.registry.clone(), r.backend.clone());
        for i in 0..10 {
            r.bus.publish(closed_event(&format!("garage/ws/noise{i}")));
        }
        until(|| async {
            r.registry.get_closed("garage/ws/lost1").await.is_some()
                && r.registry.get_closed("garage/ws/lost2").await.is_some()
        })
        .await;
        let lost = r.registry.get_closed("garage/ws/lost1").await.unwrap();
        assert_eq!(
            (
                lost.claude_session_id.as_deref(),
                lost.reason.as_str(),
                lost.dir.as_str()
            ),
            (Some("conv-a"), "crash", "/w")
        );
        until(|| async { r.registry.get_closed("garage/ws/back").await.is_none() }).await;
        assert!(
            r.registry.get_closed("garage/ws/back").await.is_none(),
            "a live session has no closed record"
        );
    }

    #[tokio::test]
    async fn reconcile_does_nothing_when_the_backend_cannot_be_listed() {
        let r = rig(8);
        r.registry.upsert_workspace("ws", "/w").await.unwrap();
        meta(&r, "garage/ws/a", "c").await;
        r.backend
            .fail_list
            .store(true, std::sync::atomic::Ordering::SeqCst);
        reconcile(&r.registry, r.backend.as_ref()).await;
        assert!(r.registry.list_closed("ws").await.is_empty());
    }
}
