//! Seam integration: router + real stores + real adapters over an isolated
//! tmux server, a temp GARAGE_DIR / claude home and a stub agents command.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use garage_protocol::SessionStatus;
use tower::ServiceExt;

use crate::backend::SessionBackend;
use crate::bus::DaemonEvent;
use crate::poller::{AgentSource, CommandAgents};
use crate::test_support::TestTmux;
use crate::{app, AppState, Config};

#[tokio::test]
async fn notification_hook_flows_through_real_adapters() {
    let tmux = TestTmux::new();
    let garage = tempfile::tempdir().unwrap();
    let claude_home = tempfile::tempdir().unwrap();
    let backend = Arc::new(tmux.backend());
    backend.init().await.unwrap();
    backend
        .create("garage/ws/a", tmux.dir(), "sleep", &["60".to_string()], &[])
        .await
        .unwrap();

    // Stub `claude agents --json`: no agents, so resolution falls back to cwd.
    let agents: Arc<dyn AgentSource> =
        Arc::new(CommandAgents::new("sh", vec!["-c".into(), "echo '[]'".into()]));
    let config = Config {
        port: 0,
        garage_dir: garage.path().to_path_buf(),
        tmux_socket: Some(tmux.socket().to_owned()),
        ui_origins: vec![],
    };
    let state = AppState::with_agents(
        config,
        backend.clone(),
        Arc::new(crate::os::UnsupportedOs),
        agents.clone(),
        claude_home.path().to_path_buf(),
    );
    let mut events = state.bus.subscribe();

    // The hook token comes from the real registry (persisted under GARAGE_DIR).
    let token = state.registry.get_hook_token().await.unwrap();
    assert!(garage.path().join("state.json").exists());

    let cwd = backend.list().await.unwrap()[0].dir.clone();
    let body = serde_json::json!({
        "hook_event_name": "Notification",
        "message": "Claude needs your permission to use Bash",
        "session_id": "unknown-claude-session",
        "cwd": cwd,
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/hooks/claude?token={token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = app(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let entry = state.status.entry("garage/ws/a");
    assert_eq!(entry.state, SessionStatus::NeedsInput);
    assert_eq!(entry.message.as_deref(), Some("Claude needs your permission to use Bash"));
    match events.try_recv().unwrap() {
        DaemonEvent::StatusTransition { id, to, .. } => {
            assert_eq!(id, "garage/ws/a");
            assert_eq!(to, SessionStatus::NeedsInput);
        }
        other => panic!("unexpected event {other:?}"),
    }

    // Poller over the real sink: once the session dies, its status is dropped.
    let poller = state.poller(agents);
    poller.poll_once().await;
    backend.kill("garage/ws/a").await.unwrap();
    poller.poll_once().await;
    assert!(!state.status.has("garage/ws/a"));
}
