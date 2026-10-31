//! Port of the subscription half of `daemon/src/notify.js`: a desktop
//! notification on a transition INTO `needs-input`, edge-triggered (the bus
//! only carries real transitions), and only when no UI client reported itself
//! visible within the last 45 seconds (`api::visibility`). A failing or
//! unsupported notification is swallowed: it must never affect the daemon.

use std::sync::Arc;

use garage_protocol::SessionStatus;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use super::Os;
use crate::api::visibility::Visibility;
use crate::bus::{DaemonEvent, EventBus};

pub const TITLE: &str = "claude-garage";

pub fn message_for(id: &str) -> String {
    format!("{id} needs input")
}

/// Handle one event; returns whether a notification was attempted.
pub async fn handle(event: &DaemonEvent, os: &dyn Os, visibility: &Visibility) -> bool {
    let DaemonEvent::StatusTransition {
        id,
        to: SessionStatus::NeedsInput,
        ..
    } = event
    else {
        return false;
    };
    if visibility.any_visible() {
        return false;
    }
    let _ = os.notify(TITLE, &message_for(id)).await;
    true
}

/// Subscribe now (so no event between this call and the first poll is lost)
/// and notify in a background task until the bus closes.
pub fn spawn(bus: &EventBus, os: Arc<dyn Os>, visibility: Visibility) -> JoinHandle<()> {
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    handle(&event, os.as_ref(), &visibility).await;
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::fake::FakeOs;

    fn transition(id: &str, to: SessionStatus) -> DaemonEvent {
        DaemonEvent::StatusTransition {
            id: id.into(),
            from: SessionStatus::Working,
            to,
            since: Some(1),
        }
    }

    #[tokio::test]
    async fn needs_input_with_no_visible_client_notifies_once_with_the_session_id() {
        let os = FakeOs::default();
        let vis = Visibility::default();
        assert!(
            handle(
                &transition("garage/ws/a", SessionStatus::NeedsInput),
                &os,
                &vis
            )
            .await
        );
        assert_eq!(
            os.notifications(),
            [(
                "claude-garage".to_string(),
                "garage/ws/a needs input".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn other_transitions_and_sessions_changed_never_notify() {
        let os = FakeOs::default();
        let vis = Visibility::default();
        for to in [
            SessionStatus::Working,
            SessionStatus::Idle,
            SessionStatus::Done,
        ] {
            assert!(!handle(&transition("garage/ws/a", to), &os, &vis).await);
        }
        assert!(!handle(&DaemonEvent::SessionsChanged, &os, &vis).await);
        assert!(os.notifications().is_empty());
    }

    #[tokio::test]
    async fn a_visible_wall_suppresses_the_notification() {
        let os = FakeOs::default();
        let vis = Visibility::default();
        vis.record("tui", true);
        assert!(
            !handle(
                &transition("garage/ws/a", SessionStatus::NeedsInput),
                &os,
                &vis
            )
            .await
        );
        assert!(os.notifications().is_empty());
        vis.record("tui", false);
        assert!(
            handle(
                &transition("garage/ws/a", SessionStatus::NeedsInput),
                &os,
                &vis
            )
            .await
        );
    }

    #[tokio::test]
    async fn an_unsupported_os_is_swallowed_silently() {
        let os = FakeOs::unsupported();
        assert!(
            handle(
                &transition("garage/ws/a", SessionStatus::NeedsInput),
                &os,
                &Visibility::default()
            )
            .await
        );
    }

    #[tokio::test]
    async fn spawned_task_notifies_from_the_bus() {
        let bus = EventBus::default();
        let os = Arc::new(FakeOs::default());
        let task = spawn(&bus, os.clone(), Visibility::default());
        bus.publish(transition("garage/ws/b", SessionStatus::NeedsInput));
        for _ in 0..100 {
            if !os.notifications().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(os.notifications().len(), 1);
        task.abort();
    }
}
