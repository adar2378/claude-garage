//! Test rig for the plugin host: an `AppState` over temp dirs and a fake
//! backend, a recording runner, and helpers to install plugins and call the
//! API. Never touches the real `~/.garage`, `~/.claude` or any real tmux.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use super::layout::{Entry, Paths, PluginsFile, Source};
use super::runner::{RunOutcome, RunSpec, Runner};
use crate::bus::DaemonEvent;
use crate::poller::fakes::{FakeAgents, FakeBackend};
use crate::state::AppState;
use crate::Config;

/// A runner that records every run and can hold runs open.
#[derive(Default)]
pub struct FakeRunner {
    pub calls: Mutex<Vec<Call>>,
    pub outcomes: Mutex<VecDeque<RunOutcome>>,
    /// When set, a run waits for a permit before it finishes.
    pub gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    active: AtomicUsize,
    pub max_active: AtomicUsize,
}

#[derive(Clone)]
pub struct Call {
    pub spec: RunSpec,
    /// Content of the file `GARAGE_CONTEXT_FILE` named, read during the run.
    pub ctx_file: Option<String>,
    /// Unix mode bits of that file while the run was going.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub ctx_file_mode: Option<u32>,
}

impl Call {
    pub fn env(&self, key: &str) -> Option<&str> {
        self.spec.env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// The context JSON the plugin would read from stdin.
    pub fn ctx(&self) -> Value {
        serde_json::from_slice(&self.spec.stdin).expect("context is JSON")
    }
}

impl FakeRunner {
    pub fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn call(&self, i: usize) -> Call {
        self.calls.lock().unwrap()[i].clone()
    }

    pub fn hold(&self) -> Arc<tokio::sync::Semaphore> {
        let g = Arc::new(tokio::sync::Semaphore::new(0));
        *self.gate.lock().unwrap() = Some(g.clone());
        g
    }
}

#[async_trait]
impl Runner for FakeRunner {
    async fn run(&self, spec: RunSpec) -> RunOutcome {
        let ctx_file = spec.env.iter().find(|(k, _)| k == "GARAGE_CONTEXT_FILE").and_then(|(_, p)| std::fs::read_to_string(p).ok());
        #[cfg(unix)]
        let ctx_file_mode = spec
            .env
            .iter()
            .find(|(k, _)| k == "GARAGE_CONTEXT_FILE")
            .and_then(|(_, p)| std::fs::metadata(p).ok())
            .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777);
        #[cfg(not(unix))]
        let ctx_file_mode = None;
        self.calls.lock().unwrap().push(Call { spec, ctx_file, ctx_file_mode });
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(now, Ordering::SeqCst);
        let gate = self.gate.lock().unwrap().clone();
        if let Some(g) = gate {
            g.acquire().await.unwrap().forget();
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.outcomes.lock().unwrap().pop_front().unwrap_or(RunOutcome::Success)
    }
}

pub struct Rig {
    pub garage: tempfile::TempDir,
    pub src: tempfile::TempDir,
    pub st: AppState,
    pub backend: Arc<FakeBackend>,
    pub runner: Arc<FakeRunner>,
    pub rx: tokio::sync::broadcast::Receiver<DaemonEvent>,
    _claude_home: tempfile::TempDir,
}

impl Rig {
    /// Fake runner, fake backend.
    pub fn new() -> Self {
        Self::build(true)
    }

    /// Real processes (tiny scripts the test writes), fake backend.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub fn real() -> Self {
        Self::build(false)
    }

    fn build(fake: bool) -> Self {
        let garage = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::new());
        let config = Config { port: 4747, garage_dir: garage.path().to_path_buf(), tmux_socket: None, tmux_control: false, ui_origins: vec![] };
        let claude_home = tempfile::tempdir().unwrap();
        let mut st = AppState::with_agents(
            config,
            backend.clone(),
            Arc::new(crate::os::UnsupportedOs),
            Arc::new(FakeAgents(Default::default())),
            claude_home.path().to_path_buf(),
        );
        let runner = Arc::new(FakeRunner::default());
        if fake {
            st = st.with_plugin_runner(runner.clone());
        }
        let rx = st.bus.subscribe();
        Self { garage, src: tempfile::tempdir().unwrap(), st, backend, runner, rx, _claude_home: claude_home }
    }

    pub fn paths(&self) -> Paths {
        Paths::new(self.garage.path())
    }

    /// Register a linked plugin whose manifest is `toml` and approve its
    /// permissions, as `plugin link` does after the user says yes; returns its
    /// dir.
    pub fn link(&self, id: &str, toml: &str) -> PathBuf {
        let granted = garage_plugin::parse(toml).map(|m| m.permission_names()).unwrap_or_default();
        self.register(id, toml, Source::Link { path: self.src.path().join(id).to_string_lossy().into_owned() }, Some(granted))
    }

    /// Register a linked plugin nobody has approved (what a hand-edited
    /// registry, or a CLI that skipped the prompt, leaves behind).
    pub fn link_unapproved(&self, id: &str, toml: &str) -> PathBuf {
        self.register(id, toml, Source::Link { path: self.src.path().join(id).to_string_lossy().into_owned() }, None)
    }

    /// Register a first-party plugin shipped with garage (no approval needed).
    pub fn bundle(&self, id: &str, toml: &str) -> PathBuf {
        self.register(id, toml, Source::Bundled { path: self.src.path().join(id).to_string_lossy().into_owned() }, None)
    }

    fn register(&self, id: &str, toml: &str, source: Source, granted: Option<Vec<String>>) -> PathBuf {
        let dir = self.src.path().join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("garage-plugin.toml"), toml).unwrap();
        let paths = self.paths();
        PluginsFile::update(&paths, |file| {
            file.plugins.retain(|e| e.id != id);
            let mut entry = Entry::new(id, source);
            entry.granted_permissions = granted;
            file.plugins.push(entry);
            (true, ())
        })
        .unwrap();
        dir
    }

    pub fn set_manifest(&self, id: &str, toml: &str) {
        std::fs::write(self.src.path().join(id).join("garage-plugin.toml"), toml).unwrap();
    }

    /// Register a first-party plugin and load it.
    pub fn install_bundled(&self, id: &str, toml: &str) -> PathBuf {
        let dir = self.bundle(id, toml);
        self.st.plugins.reload().unwrap();
        dir
    }

    /// Register an unapproved linked plugin and load it.
    pub fn install_unapproved(&self, id: &str, toml: &str) -> PathBuf {
        let dir = self.link_unapproved(id, toml);
        self.st.plugins.reload().unwrap();
        dir
    }

    /// Link and load a plugin in one go.
    pub fn install(&self, id: &str, toml: &str) -> PathBuf {
        let dir = self.link(id, toml);
        self.st.plugins.reload().unwrap();
        dir
    }

    pub fn token(&self, id: &str) -> String {
        std::fs::read_to_string(self.paths().token(id)).unwrap().trim().to_owned()
    }

    pub fn events(&mut self) -> Vec<DaemonEvent> {
        let mut out = vec![];
        while let Ok(e) = self.rx.try_recv() {
            out.push(e);
        }
        out
    }

    /// The per-install UI token, as the TUI reads it from disk.
    pub fn ui_token(&self) -> String {
        std::fs::read_to_string(super::layout::ui_token_path(self.garage.path())).unwrap().trim().to_owned()
    }

    /// A call the TUI would make: the UI token as bearer, no plugin token.
    pub async fn call_ui(&self, method: &str, uri: &str, body: Option<&str>) -> (StatusCode, axum::http::HeaderMap, Value) {
        let t = self.ui_token();
        self.call(method, uri, Some(&t), body).await
    }

    pub async fn call(&self, method: &str, uri: &str, token: Option<&str>, body: Option<&str>) -> (StatusCode, axum::http::HeaderMap, Value) {
        self.call_with(method, uri, token, body, &[]).await
    }

    pub async fn call_with(
        &self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<&str>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let req = req.body(Body::from(body.unwrap_or("").to_owned())).unwrap();
        let res = crate::app(self.st.clone()).oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, headers, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
}

/// A manifest with the given permissions and extra TOML after the header.
pub fn manifest(id: &str, perms: &[&str], extra: &str) -> String {
    let perms = perms.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ");
    format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\napi_version = 1\nplatforms = [\"macos\", \"linux\", \"windows\"]\npermissions = [{perms}]\n{extra}\n"
    )
}

/// Poll `f` every 20 ms for up to 5 s.
pub async fn wait_for(f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for condition");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

impl Rig {
    /// Real tmux backend (the caller owns the isolated `TestTmux`).
    pub fn with_tmux(tmux: &crate::test_support::TestTmux) -> Self {
        let garage = tempfile::tempdir().unwrap();
        let config = Config {
            port: 4747,
            garage_dir: garage.path().to_path_buf(),
            tmux_socket: Some(tmux.socket().to_owned()),
            tmux_control: false,
            ui_origins: vec![],
        };
        let claude_home = tempfile::tempdir().unwrap();
        let st = AppState::with_agents(
            config,
            Arc::new(tmux.backend()),
            Arc::new(crate::os::UnsupportedOs),
            Arc::new(FakeAgents(Default::default())),
            claude_home.path().to_path_buf(),
        );
        let rx = st.bus.subscribe();
        Self {
            garage,
            src: tempfile::tempdir().unwrap(),
            st,
            backend: Arc::new(FakeBackend::new()),
            runner: Arc::new(FakeRunner::default()),
            rx,
            _claude_home: claude_home,
        }
    }

    /// Scale timer intervals (5 "seconds" become `5 * unit`).
    pub fn fast_timers(&mut self, unit: Duration) {
        let lookup = Arc::new(super::context::StateLookup {
            backend: self.st.backend.clone(),
            status: self.st.status.clone(),
            registry: self.st.registry.clone(),
        });
        self.st.plugins = super::PluginHost::with_runner(&self.st.config, self.st.bus.clone(), lookup, self.runner.clone())
            .with_timer_unit(unit);
        self.st.plugins.set_backend(self.st.backend.clone());
    }

    /// The next bus event matching `pred`, within 5 s.
    pub async fn next_event(&mut self, pred: impl Fn(&DaemonEvent) -> bool) -> DaemonEvent {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let left = end.saturating_duration_since(Instant::now());
            let ev = tokio::time::timeout(left, self.rx.recv()).await.expect("event in time").expect("bus open");
            if pred(&ev) {
                return ev;
            }
        }
    }
}
