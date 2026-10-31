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

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::middleware;

pub use config::Config;

/// The full application: routes plus the origin guard.
pub fn app(config: &Config) -> axum::Router {
    api::routes().layer(middleware::from_fn_with_state(
        Arc::new(config.ui_origins.clone()),
        security::origin_guard,
    ))
}

/// Bind 127.0.0.1 and serve until ctrl-c. A taken port is an error naming the
/// port and `GARAGE_PORT`.
pub async fn run(config: Config) -> Result<()> {
    let addr = ("127.0.0.1", config.port);
    let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| {
        format!("cannot bind 127.0.0.1:{}; is it taken? set GARAGE_PORT to use another port", config.port)
    })?;
    axum::serve(listener, app(&config))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
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

    async fn call(cfg: &Config, method: &str, uri: &str, origin: Option<&str>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(o) = origin {
            req = req.header("origin", o);
        }
        let res = app(cfg).oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn health_reports_status_version_and_pid() {
        let (s, body) = call(&config(&[]), "GET", "/api/health", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["pid"], std::process::id());
        assert!(body["version"].is_string());
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

    #[tokio::test]
    async fn spec_routes_exist_and_diff_route_is_404() {
        let c = config(&[]);
        for (m, u) in [
            ("GET", "/api/events"), ("GET", "/api/sessions"), ("POST", "/api/sessions/restore"),
            ("POST", "/api/sessions/restart"), ("DELETE", "/api/sessions/garage%2Fws%2Fa"),
            ("PUT", "/api/workspaces"), ("PATCH", "/api/workspaces/ws"), ("DELETE", "/api/workspaces/ws"),
            ("POST", "/api/hooks/claude"), ("GET", "/api/usage"), ("POST", "/api/worktrees/finish"),
        ] {
            assert_eq!(call(&c, m, u, None).await.0, StatusCode::NOT_IMPLEMENTED, "{m} {u}");
        }
        let (s, body) = call(&c, "GET", "/api/diff/ws", None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert!(body["error"].is_string());
    }
}
