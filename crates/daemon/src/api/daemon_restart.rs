//! Port of `daemon/src/daemon-restart.js`: `POST /api/daemon/restart` and the
//! successor's port-handoff wait.
//!
//! The route starts a detached successor (this same binary, `daemon --port N`,
//! `GARAGE_PREDECESSOR_PID` set, logging to `<GARAGE_DIR>/daemon.log`), answers
//! 202 `{pid}`, then asks `run()` to shut down. The successor waits for the
//! predecessor's `/api/health` to stop answering before it binds the port.
//! tmux sessions are untouched.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use super::hooks::internal;
use crate::state::AppState;

/// How long a successor waits for its predecessor to let go of the port.
pub const PREDECESSOR_DEADLINE_MS: u64 = 10_000;
const POLL_INTERVAL_MS: u64 = 200;

/// Process-lifecycle handle in `AppState`: `run()` waits on `shutdown`
/// alongside ctrl-c, and the restart route fires it. `successor` overrides the
/// command the route spawns (tests; the default is this executable).
#[derive(Clone, Default)]
pub struct Lifecycle {
    pub shutdown: Arc<Notify>,
    successor: Option<(PathBuf, Vec<String>)>,
}

impl Lifecycle {
    pub fn with_successor(mut self, exe: impl Into<PathBuf>, args: Vec<String>) -> Self {
        self.successor = Some((exe.into(), args));
        self
    }

    fn successor_cmd(&self, port: u16) -> std::io::Result<(PathBuf, Vec<String>)> {
        if let Some(s) = &self.successor {
            return Ok(s.clone());
        }
        // Under `cargo test` current_exe() is the test binary: re-running it
        // detached would re-run the suite.
        if cfg!(test) {
            return Err(std::io::Error::other(
                "no successor command configured in tests",
            ));
        }
        Ok((std::env::current_exe()?, default_successor_args(port)))
    }
}

/// `--port` carries the listening port so a daemon started with
/// `claude-garage daemon --port X` is replaced by one on X.
pub fn default_successor_args(port: u16) -> Vec<String> {
    vec!["daemon".into(), "--port".into(), port.to_string()]
}

/// Starts the successor detached and returns its pid. Same `daemon.log` as the
/// launcher's detached start; honours `GARAGE_DIR` via `garage_dir` so a
/// scratch run never writes into the real `~/.garage`.
pub fn spawn_successor(
    exe: &Path,
    args: &[String],
    garage_dir: &Path,
    predecessor_pid: u32,
) -> std::io::Result<u32> {
    std::fs::create_dir_all(garage_dir)?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(garage_dir.join("daemon.log"))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args)
        .env("GARAGE_PREDECESSOR_PID", predecessor_pid.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // JS `detached` is setsid: a new session, so closing the predecessor's
        // terminal (SIGHUP) or its process group never reaches the successor.
        // SAFETY: the pre_exec closure runs between fork and exec and calls
        // only `setsid`, which is async-signal-safe and touches no memory.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let child = cmd.spawn()?;
    let pid = child.id();
    // Reap it if it exits while this daemon is still alive.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(pid)
}

pub async fn restart(State(state): State<AppState>) -> Response {
    let port = state.config.port;
    let spawned = state.lifecycle.successor_cmd(port).and_then(|(exe, args)| {
        spawn_successor(&exe, &args, &state.config.garage_dir, std::process::id())
    });
    let pid = match spawned {
        Ok(pid) => pid,
        Err(e) => return internal(e.into()),
    };
    // The reply must reach the client before this process tears down: the
    // graceful shutdown lets the in-flight response finish, and the small
    // delay covers the flush (JS: `setImmediate`).
    let shutdown = state.lifecycle.shutdown.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown.notify_one();
    });
    (StatusCode::ACCEPTED, Json(json!({ "pid": pid }))).into_response()
}

/// True while `GET /api/health` on `127.0.0.1:port` still answers 2xx. A short
/// timeout keeps one failed connect from stretching the poll interval. Raw TCP
/// instead of an HTTP client dependency.
pub async fn default_probe(port: u16) -> bool {
    let attempt = async {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .ok()?;
        s.write_all(format!("GET /api/health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes())
            .await
            .ok()?;
        // Only the status line matters; never wait for the body or EOF.
        let mut line = String::new();
        tokio::io::BufReader::new(s.take(1024))
            .read_line(&mut line)
            .await
            .ok()?;
        let status = line.split_whitespace().nth(1)?;
        Some(status.starts_with('2'))
    };
    tokio::time::timeout(Duration::from_millis(500), attempt)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// Polls until `probe()` says the predecessor is gone (port free, safe to
/// bind) or `deadline_ms` passes, in which case it errors naming the port and
/// pid so the successor fails loud instead of racing for the port. `probe`,
/// `sleep` (ms) and `now` (ms) are injectable so tests need no real daemon or
/// time.
pub async fn wait_for_predecessor<P, PF, S, SF, N>(
    port: u16,
    pid: Option<u32>,
    mut probe: P,
    mut sleep: S,
    mut now: N,
    deadline_ms: u64,
) -> Result<(), String>
where
    P: FnMut() -> PF,
    PF: Future<Output = bool>,
    S: FnMut(u64) -> SF,
    SF: Future<Output = ()>,
    N: FnMut() -> u64,
{
    let start = now();
    while now().saturating_sub(start) < deadline_ms {
        if !probe().await {
            return Ok(());
        }
        sleep(POLL_INTERVAL_MS).await;
    }
    let who = pid.map_or("unknown pid".to_owned(), |p| format!("pid {p}"));
    Err(format!(
        "predecessor daemon ({who}) did not release port {port} within {deadline_ms}ms"
    ))
}

/// Boot-time handoff with real time and the real probe.
pub async fn wait_for_predecessor_real(port: u16, pid: Option<u32>) -> Result<(), String> {
    let t0 = std::time::Instant::now();
    wait_for_predecessor(
        port,
        pid,
        || default_probe(port),
        |ms| tokio::time::sleep(Duration::from_millis(ms)),
        move || t0.elapsed().as_millis() as u64,
        PREDECESSOR_DEADLINE_MS,
    )
    .await
}
