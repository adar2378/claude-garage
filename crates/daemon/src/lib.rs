//! claude-garage daemon (p21-rust-daemon): a port of the Fastify daemon in
//! `daemon/src/*.js`. Scaffold only: the router, config, event bus and the
//! `SessionBackend` / `Os` seams exist; behaviour is ported module by module.

pub mod api;
pub mod backend;
pub mod bus;
pub mod config;
pub mod error;
pub mod git;
pub mod install;
pub mod os;
pub mod poller;
pub mod security;
pub mod session_resolve;
pub mod state;
pub mod transcript;

#[cfg(test)]
mod lane_f_tests;
#[cfg(test)]
mod wiring_tests;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::middleware;
use backend::SessionBackend;

pub use config::Config;
pub use state::AppState;

/// The full application: routes plus the origin guard.
pub fn app(state: AppState) -> axum::Router {
    api::routes()
        .layer(middleware::from_fn_with_state(state.clone(), security::origin_guard))
        .with_state(state)
}

/// Bind 127.0.0.1 only. A taken port is an error naming the port and
/// `GARAGE_PORT`.
pub async fn bind(config: &Config) -> Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(("127.0.0.1", config.port)).await.with_context(|| {
        format!("cannot bind 127.0.0.1:{}; is it taken? set GARAGE_PORT to use another port", config.port)
    })
}

/// How long open connections (SSE streams never drain) get after shutdown
/// starts before the server is dropped anyway (JS `forceCloseConnections`).
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

/// Serve on 127.0.0.1 until ctrl-c or `POST /api/daemon/restart`.
pub async fn run(config: Config) -> Result<()> {
    // A successor spawned by the restart route carries GARAGE_PREDECESSOR_PID:
    // wait for that daemon's port to free before binding, failing loud if it
    // does not (no race for the port).
    if let Some(raw) = std::env::var("GARAGE_PREDECESSOR_PID").ok().filter(|s| !s.is_empty()) {
        let pid = raw.parse::<u32>().unwrap_or(0);
        api::daemon_restart::wait_for_predecessor_real(config.port, pid).await.map_err(anyhow::Error::msg)?;
    }
    let listener = bind(&config).await?;
    let backend = Arc::new(backend::tmux::TmuxBackend::new(config.tmux_socket.clone()));
    backend.init().await.context("initialising the tmux server")?;
    let agents: Arc<dyn poller::AgentSource> = Arc::new(poller::CommandAgents::default());
    let state = AppState::with_agents(config, backend, os::platform(), agents.clone(), install::claude_home_from_env());
    let poller = state.poller(agents).start();
    let notifier = os::notify::spawn(&state.bus, state.os.clone(), state.visibility.clone());
    let shutdown = state.lifecycle.shutdown.clone();
    let (begin_tx, begin_rx) = tokio::sync::oneshot::channel::<()>();
    let serve = axum::serve(listener, app(state))
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = shutdown.notified() => {}
            }
            let _ = begin_tx.send(());
        })
        .into_future();
    tokio::pin!(serve);
    let result = tokio::select! {
        r = &mut serve => r.map_err(anyhow::Error::from),
        _ = async { let _ = begin_rx.await; tokio::time::sleep(SHUTDOWN_GRACE).await } => Ok(()),
    };
    poller.abort();
    notifier.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    fn config(origins: &[&str]) -> Config {
        Config {
            port: 0,
            garage_dir: "/nonexistent".into(),
            tmux_socket: None,
            ui_origins: origins.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn state(cfg: Config) -> AppState {
        AppState::new(cfg, Arc::new(backend::tmux::TmuxBackend::new(None)), Arc::new(os::UnsupportedOs))
    }

    async fn call(cfg: &Config, method: &str, uri: &str, origin: Option<&str>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(o) = origin {
            req = req.header("origin", o);
        }
        let res = app(state(cfg.clone())).oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// The launcher's stale-daemon gate compares /api/health's version with
    /// package.json's, so they must never drift.
    #[tokio::test]
    async fn health_reports_status_pid_and_the_package_json_version() {
        let (s, body) = call(&config(&[]), "GET", "/api/health", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["pid"], std::process::id());
        let pkg: Value = serde_json::from_str(include_str!("../../../package.json")).unwrap();
        assert_eq!(body["version"], pkg["version"]);
    }

    #[tokio::test]
    async fn foreign_origin_is_refused_on_writes_but_not_reads_or_headerless() {
        let c = config(&[]);
        let (s, body) = call(&c, "POST", "/api/sessions", Some("https://evil.example")).await;
        assert_eq!((s, body), (StatusCode::FORBIDDEN, json!({"error": "origin not allowed"})));
        // GET is never guarded; a header-less write reaches the (stub) route.
        assert_eq!(call(&c, "GET", "/api/health", Some("https://evil.example")).await.0, StatusCode::OK);
        assert_eq!(call(&c, "POST", "/api/sessions", None).await.0, StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn listed_origin_passes_the_guard() {
        let c = config(&["http://ok.test"]);
        assert_eq!(call(&c, "DELETE", "/api/sessions/x", Some("http://ok.test")).await.0, StatusCode::NOT_IMPLEMENTED);
    }

    /// 405 means the path is registered (for another method); 404 means it is
    /// not routed at all. Together they tell a real route from a typo, which a
    /// bare 404 check cannot.
    #[tokio::test]
    async fn registered_paths_answer_405_for_other_methods_and_removed_diff_route_is_404() {
        let c = config(&[]);
        for (m, u) in [
            ("PUT", "/api/events"), ("PATCH", "/api/sessions"), ("GET", "/api/sessions/restore"),
            ("GET", "/api/sessions/restart"), ("GET", "/api/sessions/x"), ("POST", "/api/workspaces/ws"),
            ("GET", "/api/hooks/claude"), ("POST", "/api/usage"), ("GET", "/api/worktrees/finish"),
            ("GET", "/api/daemon/restart"),
        ] {
            assert_eq!(call(&c, m, u, None).await.0, StatusCode::METHOD_NOT_ALLOWED, "{m} {u}");
        }
        for u in ["/api/diff/ws", "/api/sesions"] {
            let (s, body) = call(&c, "GET", u, None).await;
            assert_eq!((s, body), (StatusCode::NOT_FOUND, json!({"error": "not found"})), "{u}");
        }
    }

    #[tokio::test]
    async fn binds_loopback_only() {
        let listener = bind(&config(&[])).await.unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());
    }
}
