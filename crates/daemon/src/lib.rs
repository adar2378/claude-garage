//! claude-garage daemon (p21-rust-daemon): a port of the Fastify daemon in
//! `daemon/src/*.js`. Scaffold only: the router, config, event bus and the
//! `SessionBackend` / `Os` seams exist; behaviour is ported module by module.

pub mod api;
pub mod backend;
pub mod bus;
pub mod closed;
pub mod closing;
pub mod config;
pub mod control;
pub mod error;
pub mod git;
pub mod install;
pub mod os;
pub mod plugins;
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

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::middleware;

pub use config::Config;
pub use state::AppState;

/// The full application: routes plus the origin guard.
pub fn app(state: AppState) -> axum::Router {
    api::routes()
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::origin_guard,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::host_guard,
        ))
        .with_state(state)
}

/// Bind 127.0.0.1 only. A taken port is an error naming the port and
/// `GARAGE_PORT`.
pub async fn bind(config: &Config) -> Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(("127.0.0.1", config.port))
        .await
        .with_context(|| {
            format!(
                "cannot bind 127.0.0.1:{}; is it taken? set GARAGE_PORT to use another port",
                config.port
            )
        })
}

/// How long open connections (SSE streams never drain) get after shutdown
/// starts before the server is dropped anyway (JS `forceCloseConnections`).
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

static PREDECESSOR_PID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

fn read_predecessor_pid() -> Option<String> {
    std::env::var("GARAGE_PREDECESSOR_PID")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Reads `GARAGE_PREDECESSOR_PID` and removes it from the environment, so the
/// tmux server and the sessions' children never inherit it (it is not a daemon
/// setting). The binary calls this first thing in `main`, before any thread
/// exists; that is what makes the environment write sound. Without the call
/// (tests), `run` still reads the variable but leaves it in place.
pub fn take_predecessor_env() {
    PREDECESSOR_PID.get_or_init(|| {
        let raw = read_predecessor_pid();
        if raw.is_some() {
            // SAFETY: documented above: called single-threaded, before the
            // async runtime starts.
            unsafe { std::env::remove_var("GARAGE_PREDECESSOR_PID") };
        }
        raw
    });
}

fn predecessor_pid_env() -> Option<String> {
    PREDECESSOR_PID.get_or_init(read_predecessor_pid).clone()
}

/// Serve on 127.0.0.1 until ctrl-c or `POST /api/daemon/restart`.
pub async fn run(config: Config) -> Result<()> {
    // A successor spawned by the restart route carries GARAGE_PREDECESSOR_PID:
    // wait for that daemon's port to free before binding, failing loud if it
    // does not (no race for the port).
    if let Some(raw) = predecessor_pid_env() {
        let pid = raw.parse::<u32>().ok();
        api::daemon_restart::wait_for_predecessor_real(config.port, pid)
            .await
            .map_err(anyhow::Error::msg)?;
    }
    let listener = bind(&config).await?;
    // Chosen once, here: nothing else branches on the backend.
    let kind = backend::Kind::from_env_value(std::env::var("GARAGE_BACKEND").ok().as_deref())?;
    let control_on = config.tmux_control && kind == backend::Kind::Tmux;
    let control_backend = backend::tmux::TmuxBackend::new(config.tmux_socket.clone());
    let backend = backend::select(kind, &config)?;
    backend
        .init()
        .await
        .context("initialising the session backend")?;
    let agents: Arc<dyn poller::AgentSource> = Arc::new(poller::CommandAgents::default());
    let state = AppState::with_agents(
        config,
        backend.clone(),
        os::platform(),
        agents.clone(),
        install::claude_home_from_env(),
    );
    let shared = state.poller(agents);
    // tmux control mode wakes the poller on session changes; the tick stays
    // as the fallback (and carries pane titles, which tmux does not announce).
    // `GARAGE_TMUX_CONTROL=0` skips it: the plain tick runs alone. Only the
    // tmux backend has a control-mode client.
    let control = control_on.then(|| control::spawn(control_backend, shared.control_link()));
    let poller = shared.start();
    // First-party plugins: register and refresh them before the host loads.
    if let Some(root) = plugins::bundled::prepare(state.plugins.paths()) {
        match plugins::bundled::seed(state.plugins.paths(), &root) {
            Ok(s) if !s.added.is_empty() || !s.refreshed.is_empty() => {
                eprintln!(
                    "plugins: bundled from {}: added {:?}, refreshed {:?}",
                    root.display(),
                    s.added,
                    s.refreshed
                );
            }
            Ok(_) => {}
            Err(e) => eprintln!("plugins: cannot register the bundled plugins: {e}"),
        }
    }
    let plugin_host = state.plugins.start();
    let closed = closed::spawn(&state.bus, state.registry.clone(), state.backend.clone());
    let (poller_stop, control_stop) = (
        poller.abort_handle(),
        control.as_ref().map(|c| c.abort_handle()),
    );
    let lifecycle = state.lifecycle.shutdown.clone();
    let (begin_tx, begin_rx) = tokio::sync::oneshot::channel::<()>();
    // One shutdown source: ctrl-c or the restart route. The poller
    // stops first, so a successor never overlaps with this daemon's polling; then `serve` ends the SSE streams.
    let signal = async move {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = lifecycle.notified() => {}
        }
        poller_stop.abort();
        if let Some(c) = &control_stop {
            c.abort();
        }
        let _ = begin_tx.send(());
    };
    let serving = serve(listener, state, signal);
    tokio::pin!(serving);
    // Backstop: connections that still hang on after SHUTDOWN_GRACE are dropped.
    let result = tokio::select! {
        r = &mut serving => r.map_err(anyhow::Error::from),
        _ = async { let _ = begin_rx.await; tokio::time::sleep(SHUTDOWN_GRACE).await } => Ok(()),
    };
    poller.abort();
    plugin_host.abort();
    closed.abort();
    if let Some(c) = &control {
        c.abort();
    }
    result
}

/// Serve until `shutdown` resolves. Open SSE streams are told to end first,
/// otherwise graceful shutdown would wait on them forever.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: AppState,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let signal = state.clone();
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async move {
            shutdown.await;
            signal.begin_shutdown();
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    fn config(origins: &[&str]) -> Config {
        Config {
            port: 0,
            garage_dir: "/nonexistent".into(),
            tmux_socket: None,
            tmux_control: false,
            ui_origins: origins.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn state(cfg: Config) -> AppState {
        AppState::new(
            cfg,
            Arc::new(backend::tmux::TmuxBackend::new(None)),
            Arc::new(os::UnsupportedOs),
        )
    }

    async fn call(
        cfg: &Config,
        method: &str,
        uri: &str,
        origin: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(o) = origin {
            req = req.header("origin", o);
        }
        let res = app(state(cfg.clone()))
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn shutdown_ends_open_event_streams_and_serve_returns() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve(listener, state(config(&[])), async {
            let _ = stopped.await;
        }));

        let mut conn = tokio::net::TcpStream::connect(addr).await.unwrap();
        conn.write_all(b"GET /api/events HTTP/1.1\r\nhost: 127.0.0.1\r\nconnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut seen = String::new();
        let mut buf = [0u8; 1024];
        while !seen.contains(": connected") {
            let n = tokio::time::timeout(std::time::Duration::from_secs(5), conn.read(&mut buf))
                .await
                .unwrap()
                .unwrap();
            assert!(n > 0, "closed before greeting: {seen}");
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }

        stop.send(()).unwrap();
        let ended = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while conn.read(&mut buf).await.unwrap() > 0 {}
        })
        .await;
        assert!(ended.is_ok(), "stream still open after shutdown");
        let served = tokio::time::timeout(std::time::Duration::from_secs(5), server).await;
        assert!(served.expect("serve returns").unwrap().is_ok());
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
        assert_eq!(
            (s, body),
            (
                StatusCode::FORBIDDEN,
                json!({"error": "origin not allowed", "code": "origin_not_allowed"})
            )
        );
        // GET is never guarded; a header-less write reaches the (stub) route.
        assert_eq!(
            call(&c, "GET", "/api/health", Some("https://evil.example"))
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            call(&c, "POST", "/api/sessions", None).await.0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn a_foreign_host_header_is_refused_on_every_route_and_loopback_passes() {
        let mut c = config(&[]);
        c.port = 4747;
        for (m, u) in [
            ("GET", "/api/health"),
            ("GET", "/api/attach?id=garage/a/b"),
            ("POST", "/api/sessions"),
            ("GET", "/nope"),
        ] {
            let req = |host: Option<&str>| {
                let mut r = Request::builder().method(m).uri(u);
                if let Some(h) = host {
                    r = r.header("host", h);
                }
                r.body(Body::empty()).unwrap()
            };
            let res = app(state(c.clone()))
                .oneshot(req(Some("evil.example:4747")))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{m} {u}");
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"error": "host not allowed", "code": "host_not_allowed"})
            );
            let ok = app(state(c.clone()))
                .oneshot(req(Some("127.0.0.1:4747")))
                .await
                .unwrap();
            assert_ne!(ok.status(), StatusCode::FORBIDDEN, "{m} {u}");
            let none = app(state(c.clone())).oneshot(req(None)).await.unwrap();
            assert_ne!(
                none.status(),
                StatusCode::FORBIDDEN,
                "{m} {u} without a Host header"
            );
        }
    }

    /// The plugin host's routes sit behind the same Host and Origin guards as
    /// every other route, and the UI-token routes still authenticate once a
    /// loopback Host is presented.
    #[tokio::test]
    async fn plugin_routes_honour_the_host_and_origin_guards_and_ui_token_routes_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = config(&[]);
        c.port = 4747;
        c.garage_dir = dir.path().to_path_buf();
        let st = state(c.clone());
        let token = std::fs::read_to_string(dir.path().join("ui-token"))
            .unwrap()
            .trim()
            .to_owned();
        assert!(
            !token.is_empty(),
            "the plugin host writes the UI token at startup"
        );

        let routes = [
            ("GET", "/api/plugins/features"),
            ("POST", "/api/plugins/reload"),
            ("PUT", "/api/plugins/ui/slots"),
            ("POST", "/api/plugins/ui/challenge"),
            ("PUT", "/api/plugins/p/registration"),
            ("POST", "/api/plugins/p/enable"),
            ("POST", "/api/plugins/p/disable"),
            ("POST", "/api/plugins/p/bind"),
            ("DELETE", "/api/plugins/p"),
            ("PUT", "/api/plugins/p/widgets/w"),
            ("DELETE", "/api/plugins/p/widgets/w"),
            ("POST", "/api/plugins/p/actions/a/run"),
            ("POST", "/api/sessions/input"),
            ("GET", "/api/workspaces/ws/closed"),
        ];
        let send =
            |m: &str, u: &str, host: Option<&str>, origin: Option<&str>, bearer: Option<&str>| {
                let mut r = Request::builder().method(m).uri(u);
                if let Some(h) = host {
                    r = r.header("host", h);
                }
                if let Some(o) = origin {
                    r = r.header("origin", o);
                }
                if let Some(b) = bearer {
                    r = r.header("authorization", format!("Bearer {b}"));
                }
                let app = app(st.clone());
                async move { app.oneshot(r.body(Body::empty()).unwrap()).await.unwrap() }
            };
        for (m, u) in routes {
            // Foreign Host: refused before any token is looked at.
            let res = send(m, u, Some("evil.example:4747"), None, Some(&token)).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{m} {u} foreign host");
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"error": "host not allowed", "code": "host_not_allowed"}),
                "{m} {u}"
            );
            // Foreign Origin on a write: refused, even with the UI token.
            if m != "GET" {
                let res = send(
                    m,
                    u,
                    Some("127.0.0.1:4747"),
                    Some("https://evil.example"),
                    Some(&token),
                )
                .await;
                let bytes = res.into_body().collect().await.unwrap().to_bytes();
                assert_eq!(
                    serde_json::from_slice::<Value>(&bytes).unwrap(),
                    json!({"error": "origin not allowed", "code": "origin_not_allowed"}),
                    "{m} {u}"
                );
            }
        }
        // Loopback Host without a token: the route's own 401, not the guard's 403.
        let res = send(
            "POST",
            "/api/plugins/reload",
            Some("127.0.0.1:4747"),
            None,
            None,
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // Loopback Host with the UI token: the UI-token route runs.
        let res = send(
            "POST",
            "/api/plugins/reload",
            Some("127.0.0.1:4747"),
            None,
            Some(&token),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let res = send(
            "POST",
            "/api/plugins/reload",
            Some("localhost:4747"),
            None,
            Some(&token),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn listed_origin_passes_the_guard() {
        let c = config(&["http://ok.test"]);
        assert_eq!(
            call(&c, "DELETE", "/api/sessions/x", Some("http://ok.test"))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }

    /// 405 means the path is registered (for another method); 404 means it is
    /// not routed at all. Together they tell a real route from a typo, which a
    /// bare 404 check cannot.
    #[tokio::test]
    async fn registered_paths_answer_405_for_other_methods_and_removed_diff_route_is_404() {
        let c = config(&[]);
        for (m, u) in [
            ("PUT", "/api/events"),
            ("PATCH", "/api/sessions"),
            ("GET", "/api/sessions/restore"),
            ("GET", "/api/sessions/restart"),
            ("GET", "/api/sessions/x"),
            ("POST", "/api/workspaces/ws"),
            ("GET", "/api/hooks/claude"),
            ("POST", "/api/usage"),
            ("GET", "/api/worktrees/finish"),
            ("GET", "/api/daemon/restart"),
        ] {
            assert_eq!(
                call(&c, m, u, None).await.0,
                StatusCode::METHOD_NOT_ALLOWED,
                "{m} {u}"
            );
        }
        for u in ["/api/diff/ws", "/api/sesions"] {
            let (s, body) = call(&c, "GET", u, None).await;
            assert_eq!(
                (s, body),
                (
                    StatusCode::NOT_FOUND,
                    json!({"error": "not found", "code": "not_found"})
                ),
                "{u}"
            );
        }
    }

    #[tokio::test]
    async fn binds_loopback_only() {
        let listener = bind(&config(&[])).await.unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());
    }
}
