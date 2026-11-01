//! The plugin host (p24, design sections 1 to 4): loads `plugins.json`,
//! validates manifests, runs plugin commands, turns bus events into hook runs
//! and keeps the widget store. Plugins are out-of-process; nothing here runs
//! plugin code in the daemon.
//!
//! Layout: [`layout`] (disk + registry file), [`runner`] (one process),
//! [`widgets`] (pushed content), [`context`] (the context JSON), [`api`]
//! (routes and token auth). This file owns load/reload, the per-plugin run
//! queue, timers and the teardown sequence.

pub mod admin;
pub mod api;
pub mod bundled;
pub mod context;
pub mod layout;
pub mod panes;
pub mod runner;
pub mod widgets;

#[cfg(test)]
mod first_party;
#[cfg(test)]
mod rig;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use garage_plugin::argv::{self, Vars};
use garage_plugin::manifest::{
    ActionContext, DEFAULT_ACTION_TIMEOUT_SECS, DEFAULT_EVENT_TIMEOUT_SECS, EventName, Manifest,
    PanePlacement, sha256_hex,
};
use garage_plugin::{check_cli_conflicts, check_host, parse};
use garage_protocol::plugins::{
    ActionSummary, ButtonSummary, PaneSummary, PluginList, PluginSummary, PluginWidgets,
    WidgetContent, WidgetSummary, WidgetsSnapshot,
};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::bus::{DaemonEvent, EventBus};
use crate::config::Config;
use context::{CtxRequest, PluginFacts, SessionInfo, SessionLookup};
use garage_protocol::widgets::limits;
use layout::{
    Entry, MANIFEST_FILE, Paths, PluginsFile, Source, create_private_dir, ensure_token,
    ensure_ui_token, write_private,
};
use runner::{PluginLog, ProcessRunner, RunOutcome, RunSpec, Runner, Running};
use widgets::{Bucket, WidgetStore};

pub use garage_plugin::argv::resolve_program;

/// Concurrent runs per plugin; the rest wait in the queue.
pub const MAX_CONCURRENT: usize = 4;
/// Queued runs per plugin; past this the oldest queued run is dropped.
pub const QUEUE_CAP: usize = 64;
/// Consecutive failed event runs before event hooks are switched off.
pub const FAIL_LIMIT: u32 = 5;
/// Above this the context goes to a file instead of the environment.
pub const CONTEXT_ENV_MAX: usize = 32 * 1024;
pub const LINK_POLL: Duration = Duration::from_secs(2);
/// Event dispatches (context building plus queueing) running at once; past
/// this an event is dropped with a log line rather than stalling the bus.
pub const DISPATCH_INFLIGHT: usize = 256;
/// `POST /api/plugins/ui/action` per plugin (burst and sustained).
pub const UI_ACTIONS_PER_SEC: u32 = 5;
/// How often the table of open panes is checked against the backend.
pub const PANE_REFRESH: Duration = Duration::from_secs(2);

/// One registry entry plus what loading it produced.
#[derive(Clone)]
pub struct PluginState {
    pub entry: Entry,
    pub dir: PathBuf,
    /// The last manifest that validated and was approved.
    pub good: Option<Good>,
    /// Why the current manifest (or the whole plugin) is not in effect.
    pub problems: Vec<String>,
    /// Set by cross-plugin rules (cli names, fallback).
    pub refused: Option<String>,
    pub token: String,
    /// (mtime, size) of the manifest file as last examined, good or not.
    seen: Option<(SystemTime, u64)>,
}

#[derive(Clone)]
pub struct Good {
    pub manifest: Arc<Manifest>,
    pub sha: String,
}

impl PluginState {
    /// Enabled, approved, loaded and not refused. Approval is the recorded
    /// `granted_permissions`: bundled plugins get it automatically at first
    /// load, anything else only when `plugin link`/`install` writes it.
    pub fn active(&self) -> bool {
        self.entry.enabled
            && self.entry.granted_permissions.is_some()
            && self.good.is_some()
            && self.refused.is_none()
    }

    pub fn manifest(&self) -> Option<&Arc<Manifest>> {
        self.good.as_ref().map(|g| &g.manifest)
    }
}

/// A summary for `plugin list` style output.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginInfo {
    pub id: String,
    pub enabled: bool,
    pub active: bool,
    pub version: Option<String>,
    pub permissions: Vec<String>,
    pub problems: Vec<String>,
}

#[derive(Default)]
struct State {
    plugins: Vec<PluginState>,
    load_error: Option<String>,
    /// Identity of the loaded set, to publish `PluginsChanged` only on change.
    signature: String,
    /// Last toast per plugin, so a poll does not repeat itself.
    notified: HashMap<String, String>,
}

#[derive(Default)]
struct QState {
    queue: VecDeque<Job>,
    running: usize,
    streak: u32,
    events_disabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Action,
    Event,
    Timer,
}

impl JobKind {
    /// Hook runs count toward auto-disable; user-triggered actions do not.
    fn is_hook(self) -> bool {
        !matches!(self, JobKind::Action)
    }
}

struct Job {
    run_id: u64,
    kind: JobKind,
    label: String,
    action: String,
    argv: Vec<String>,
    timeout: Duration,
    trigger: &'static str,
    ctx: Value,
    _timer: Option<TimerGuard>,
}

/// Clears the "this timer entry is still running" flag when the job finishes
/// or is dropped from the queue.
struct TimerGuard(Arc<AtomicBool>);
impl Drop for TimerGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// A plugin process's environment and, when the context was too big for it, the context file.
type RunEnv = (Vec<(String, String)>, Option<PathBuf>);

pub struct Inner {
    paths: Paths,
    port: u16,
    bus: EventBus,
    lookup: Arc<dyn SessionLookup>,
    runner: Arc<dyn Runner>,
    state: Mutex<State>,
    pub widgets: WidgetStore,
    running: Arc<Running>,
    queues: Mutex<HashMap<String, QState>>,
    timers: Mutex<HashMap<String, Vec<JoinHandle<()>>>>,
    logs: Mutex<HashMap<String, Arc<PluginLog>>>,
    ui_slots: Mutex<Value>,
    /// Which walls are in view, for `ui.visible` in the run context.
    visibility: Mutex<crate::api::visibility::Visibility>,
    /// Rate limit of `PUT /api/plugins/ui/slots`.
    slots_bucket: Mutex<Bucket>,
    /// Rate limit of `POST /api/plugins/ui/action`, per plugin.
    ui_action_buckets: Mutex<HashMap<String, Bucket>>,
    /// Rate limit of `POST /api/plugins/{id}/notice`, per plugin.
    notice_buckets: Mutex<HashMap<String, Bucket>>,
    /// One lock per plugin across a pane open (singleton check, limit, create).
    pane_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The per-install credential of the TUI and CLI; empty when it could not
    /// be created (then nothing matches).
    ui_token: String,
    dispatch_permits: Arc<tokio::sync::Semaphore>,
    link_poller: Mutex<Option<JoinHandle<()>>>,
    run_seq: AtomicU64,
    /// Open plugin panes (the mark that a backend session is a pane).
    panes: Mutex<Vec<panes::PaneRec>>,
    pane_seq: AtomicU64,
    backend: std::sync::OnceLock<Arc<dyn crate::backend::SessionBackend>>,
    timer_unit: Duration,
    link_poll: Duration,
}

/// Cheap to clone; lives in `AppState.plugins`.
#[derive(Clone)]
pub struct PluginHost(pub Arc<Inner>);

impl PluginHost {
    pub fn new(config: &Config, bus: EventBus, lookup: Arc<dyn SessionLookup>) -> Self {
        Self::with_runner(config, bus, lookup, Arc::new(ProcessRunner))
    }

    pub fn with_runner(
        config: &Config,
        bus: EventBus,
        lookup: Arc<dyn SessionLookup>,
        runner: Arc<dyn Runner>,
    ) -> Self {
        let ui_token = ensure_ui_token(&config.garage_dir).unwrap_or_else(|e| {
            eprintln!("plugins: cannot create the UI token under {}: {e}; UI-only plugin routes are closed", config.garage_dir.display());
            String::new()
        });
        Self(Arc::new(Inner {
            paths: Paths::new(&config.garage_dir),
            port: config.port,
            widgets: WidgetStore::new(bus.clone()),
            bus,
            lookup,
            runner,
            state: Mutex::default(),
            running: Arc::default(),
            queues: Mutex::default(),
            timers: Mutex::default(),
            logs: Mutex::default(),
            ui_slots: Mutex::new(json!({})),
            visibility: Mutex::default(),
            slots_bucket: Mutex::new(Bucket::new(std::time::Instant::now())),
            ui_action_buckets: Mutex::default(),
            notice_buckets: Mutex::default(),
            pane_locks: Mutex::default(),
            ui_token,
            dispatch_permits: Arc::new(tokio::sync::Semaphore::new(DISPATCH_INFLIGHT)),
            link_poller: Mutex::new(None),
            run_seq: AtomicU64::new(1),
            panes: Mutex::default(),
            pane_seq: AtomicU64::new(1),
            backend: std::sync::OnceLock::new(),
            timer_unit: Duration::from_secs(1),
            link_poll: LINK_POLL,
        }))
    }

    /// Scale `every_secs` (tests: 5 s becomes milliseconds).
    #[cfg(test)]
    pub fn with_timer_unit(self, unit: Duration) -> Self {
        let mut inner = Arc::try_unwrap(self.0).ok().expect("host not shared yet");
        inner.timer_unit = unit;
        Self(Arc::new(inner))
    }

    pub fn paths(&self) -> &Paths {
        &self.0.paths
    }

    pub fn widgets(&self) -> &WidgetStore {
        &self.0.widgets
    }

    pub fn bus(&self) -> &EventBus {
        &self.0.bus
    }

    /// Load the registry, then follow the bus (event hooks) and fire
    /// `daemon.started`. The handle ends the event loop when aborted.
    pub fn start(&self) -> JoinHandle<()> {
        let _ = self.reload();
        // Subscribe before returning so nothing published afterwards is missed.
        let mut rx = self.0.bus.subscribe();
        let host = self.clone();
        tokio::spawn({
            let host = host.clone();
            async move {
                host.adopt_panes().await;
                // Liveness of panes is cached; this tick is what refreshes it.
                loop {
                    tokio::time::sleep(PANE_REFRESH).await;
                    host.refresh_panes().await;
                }
            }
        });
        tokio::spawn({
            let host = host.clone();
            async move {
                host.dispatch_event("daemon.started", json!({}), None, SessionInfo::default())
                    .await
            }
        });
        tokio::spawn(async move {
            loop {
                let ev = match rx.recv().await {
                    Ok(ev) => ev,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        host.note_lagged(missed);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                let Some((name, data)) = ev.plugin_event() else {
                    continue;
                };
                // Context building looks sessions up (backend, git); that
                // must not hold up the next bus event.
                let Ok(permit) = host.0.dispatch_permits.clone().try_acquire_owned() else {
                    eprintln!(
                        "plugins: {DISPATCH_INFLIGHT} event dispatches in flight; dropped {name}"
                    );
                    continue;
                };
                let session = data.get("id").and_then(Value::as_str).map(str::to_owned);
                let host = host.clone();
                tokio::spawn(async move {
                    let mut data = data;
                    // A status change is published by the (sync) status store,
                    // which does not know the directory: look it up here.
                    if name == "session.status_changed"
                        && let Some(info) = match session.as_deref() {
                            Some(id) => host.0.lookup.session(id).await,
                            None => None,
                        }
                        && let Some(dir) = info.dir
                    {
                        data["dir"] = Value::String(dir);
                    }
                    let fallback = SessionInfo {
                        dir: data
                            .get("dir")
                            .and_then(Value::as_str)
                            .filter(|d| !d.is_empty())
                            .map(str::to_owned),
                        status: data
                            .get("last_status")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        ..Default::default()
                    };
                    host.dispatch_event(name, data, session, fallback).await;
                    drop(permit);
                });
            }
        })
    }

    /// The host fell behind the bus and `missed` events are gone for good
    /// (hooks cannot be replayed). Say so where an author will look: stderr
    /// and every active plugin's log.
    fn note_lagged(&self, missed: u64) {
        eprintln!("plugins: event hooks fell behind the bus and missed {missed} events");
        let ids: Vec<String> = self
            .0
            .state
            .lock()
            .unwrap()
            .plugins
            .iter()
            .filter(|p| p.active())
            .map(|p| p.entry.id.clone())
            .collect();
        for id in ids {
            let log = self.0.log_for(&id);
            tokio::spawn(async move {
                log.append("host", &format!("missed {missed} bus events (the host fell behind); hooks were not run for them")).await
            });
        }
    }

    // ------------------------------------------------------------------
    // Loading
    // ------------------------------------------------------------------

    /// `POST /api/plugins/reload` and daemon start: re-read `plugins.json` and
    /// every manifest. Resets the auto-disable of event hooks.
    pub fn reload(&self) -> Result<(), String> {
        self.0.reload(true)
    }

    /// Re-read linked manifests whose file changed (every 2 s while any
    /// linked plugin exists). Returns whether anything was reloaded.
    pub fn poll_links(&self) -> bool {
        self.0.poll_links()
    }

    pub fn load_error(&self) -> Option<String> {
        self.0.state.lock().unwrap().load_error.clone()
    }

    pub fn infos(&self) -> Vec<PluginInfo> {
        self.0
            .state
            .lock()
            .unwrap()
            .plugins
            .iter()
            .map(|p| PluginInfo {
                id: p.entry.id.clone(),
                enabled: p.entry.enabled,
                active: p.active(),
                version: p.manifest().map(|m| m.version.clone()),
                permissions: p
                    .manifest()
                    .map(|m| m.permission_names())
                    .unwrap_or_default(),
                problems: p
                    .problems
                    .iter()
                    .cloned()
                    .chain(p.refused.clone())
                    .collect(),
            })
            .collect()
    }

    /// `GET /api/plugins`: what the TUI binds and draws, in install order.
    /// Only an active plugin lists its actions, buttons and widgets; the rest
    /// show up bare so `plugin list` style views can say why they are off.
    pub fn list(&self) -> PluginList {
        let state = self.0.state.lock().unwrap();
        let plugins = state
            .plugins
            .iter()
            .map(|p| {
                let manifest = p.manifest().filter(|_| p.active());
                let id = &p.entry.id;
                PluginSummary {
                    id: id.clone(),
                    name: p.manifest().map_or_else(|| id.clone(), |m| m.name.clone()),
                    enabled: p.entry.enabled,
                    active: p.active(),
                    version: p.manifest().map(|m| m.version.clone()),
                    source: match p.entry.source {
                        Source::Bundled { .. } => "bundled",
                        Source::Link { .. } => "local",
                        Source::Github { .. } => "github",
                    }
                    .to_owned(),
                    permissions: p
                        .manifest()
                        .map(|m| m.permission_names())
                        .unwrap_or_default(),
                    bindings: p.entry.bindings.clone(),
                    actions: manifest
                        .map(|m| {
                            m.actions
                                .iter()
                                .map(|a| ActionSummary {
                                    id: format!("{id}:{}", a.id),
                                    title: a.title.clone(),
                                    default_key: a.default_key.clone(),
                                    contexts: a
                                        .contexts
                                        .iter()
                                        .map(|c| match c {
                                            ActionContext::Global => "global",
                                            ActionContext::Workspace => "workspace",
                                            ActionContext::Session => "session",
                                        })
                                        .map(str::to_owned)
                                        .collect(),
                                    confirm: a.confirm,
                                    pane: a.pane.clone(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    buttons: manifest
                        .map(|m| {
                            m.buttons
                                .iter()
                                .map(|b| ButtonSummary {
                                    slot: b.slot.to_protocol(),
                                    label: b.label.clone(),
                                    tooltip: b.tooltip.clone(),
                                    on_click: b.on_click.clone(),
                                    args: b.args.clone(),
                                    confirm: b.confirm,
                                    priority: b.priority,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    widgets: manifest
                        .map(|m| {
                            m.widgets
                                .iter()
                                .map(|w| WidgetSummary {
                                    id: w.id.clone(),
                                    slot: w.slot.to_protocol(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    panes: manifest
                        .map(|m| {
                            m.panes
                                .iter()
                                .map(|pane| PaneSummary {
                                    id: pane.id.clone(),
                                    title: pane.title.clone(),
                                    placement: match pane.placement {
                                        PanePlacement::Tile => "tile",
                                        PanePlacement::Overlay => "overlay",
                                        PanePlacement::Popup => "popup",
                                    }
                                    .into(),
                                    cols: pane.size.as_ref().map(|s| s.cols),
                                    rows: pane.size.as_ref().map(|s| s.rows),
                                    pct: pane.size.as_ref().is_some_and(|s| s.pct),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect();
        PluginList { plugins }
    }

    /// `GET /api/plugins/widgets[?plugin=]`: the stored content of every
    /// active plugin (or just `only`), with each plugin's revision.
    pub fn widget_snapshot(&self, only: Option<&str>) -> WidgetsSnapshot {
        let active: Vec<String> = self
            .0
            .state
            .lock()
            .unwrap()
            .plugins
            .iter()
            .filter(|p| p.active() && only.is_none_or(|o| o == p.entry.id))
            .map(|p| p.entry.id.clone())
            .collect();
        let plugins = active
            .into_iter()
            .map(|id| {
                let widgets = self
                    .0
                    .widgets
                    .widgets(&id)
                    .into_iter()
                    .map(|(name, w)| {
                        (
                            name,
                            WidgetContent {
                                slot: w.slot,
                                scopes: w.scopes,
                            },
                        )
                    })
                    .collect();
                let rev = self.0.widgets.rev(&id);
                (id, PluginWidgets { rev, widgets })
            })
            .collect();
        WidgetsSnapshot { plugins }
    }

    /// The state of one plugin, whether or not it is active.
    pub fn plugin(&self, id: &str) -> Option<PluginState> {
        self.0
            .state
            .lock()
            .unwrap()
            .plugins
            .iter()
            .find(|p| p.entry.id == id)
            .cloned()
    }

    /// The plugin, only if it is enabled, loaded and not refused.
    pub fn active(&self, id: &str) -> Option<PluginState> {
        self.plugin(id).filter(PluginState::active)
    }

    /// The plugin owning `token`, compared in constant time.
    pub fn authenticate(&self, token: &str) -> Option<PluginState> {
        // A plugin that has no token (never loaded) has an empty one: an
        // empty bearer must not match it.
        if token.is_empty() {
            return None;
        }
        let state = self.0.state.lock().unwrap();
        let mut found = None;
        for p in &state.plugins {
            if !p.token.is_empty() && crate::security::token_matches(Some(token), &p.token) {
                found = Some(p.clone());
            }
        }
        found
    }

    /// Whether `token` is the per-install UI token (constant time).
    pub fn is_ui_token(&self, token: &str) -> bool {
        !self.0.ui_token.is_empty() && crate::security::token_matches(Some(token), &self.0.ui_token)
    }

    /// The answer to a client's `ui/challenge`: proof that this daemon holds
    /// the UI token, without revealing it. `None` when no token exists.
    pub fn ui_proof(&self, port: u16, nonce: &str) -> Option<String> {
        (!self.0.ui_token.is_empty())
            .then(|| garage_protocol::auth::proof_for(&self.0.ui_token, port, nonce))
    }

    /// Enable or disable via the registry file, then reload (the teardown
    /// sequence runs when a plugin stops being active).
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        PluginsFile::update(&self.0.paths, |file| {
            match file.plugins.iter_mut().find(|e| e.id == id) {
                Some(entry) => {
                    entry.enabled = enabled;
                    (true, Ok(()))
                }
                None => (false, Err(format!("no plugin {id:?}"))),
            }
        })??;
        self.0.reload(false)
    }

    /// Rate limit one `PUT /api/plugins/ui/slots`.
    pub fn acquire_slots(&self, now: std::time::Instant) -> Result<(), Duration> {
        self.0.slots_bucket.lock().unwrap().take(now)
    }

    /// Rate limit one `POST /api/plugins/ui/action` of `plugin`: at most
    /// [`UI_ACTIONS_PER_SEC`] a second (the TUI would otherwise be asked to
    /// move its focus as fast as a plugin can post).
    pub fn acquire_ui_action(&self, plugin: &str, now: std::time::Instant) -> Result<(), Duration> {
        let rate = f64::from(UI_ACTIONS_PER_SEC);
        self.0
            .ui_action_buckets
            .lock()
            .unwrap()
            .entry(plugin.to_owned())
            .or_insert_with(|| Bucket::with_limits(now, rate, rate))
            .take(now)
    }

    /// Rate limit one `POST /api/plugins/{id}/notice`: a toast a second, in
    /// bursts of at most three (a plugin cannot flood the user's screen).
    pub fn acquire_notice(&self, plugin: &str, now: std::time::Instant) -> Result<(), Duration> {
        self.0
            .notice_buckets
            .lock()
            .unwrap()
            .entry(plugin.to_owned())
            .or_insert_with(|| {
                Bucket::with_limits(
                    now,
                    f64::from(limits::NOTICES_PER_SEC),
                    f64::from(limits::NOTICE_BURST),
                )
            })
            .take(now)
    }

    pub fn set_ui_slots(&self, slots: Value) {
        let changed = {
            let mut cur = self.0.ui_slots.lock().unwrap();
            let changed = *cur != slots;
            *cur = slots;
            changed
        };
        if changed {
            self.0.bus.publish(DaemonEvent::PluginsChanged);
        }
    }

    /// Share the daemon's visibility tracker, so runs can see whether a wall
    /// is in view.
    pub fn set_visibility(&self, visibility: crate::api::visibility::Visibility) {
        *self.0.visibility.lock().unwrap() = visibility;
    }

    pub fn ui_visible(&self) -> bool {
        self.0.visibility.lock().unwrap().any_visible()
    }

    pub fn ui_slots(&self) -> Value {
        self.0.ui_slots.lock().unwrap().clone()
    }

    // ------------------------------------------------------------------
    // Triggering
    // ------------------------------------------------------------------

    /// `POST /api/plugins/{id}/actions/{action}/run`: queue one run and return
    /// its id at once.
    pub async fn run_action(
        &self,
        plugin: &str,
        action: &str,
        body: &Value,
    ) -> Result<u64, RunError> {
        let state = self.plugin(plugin).ok_or(RunError::UnknownPlugin)?;
        if !state.active() {
            return Err(RunError::Disabled);
        }
        let manifest = state.manifest().cloned().ok_or(RunError::Disabled)?;
        let a = manifest.action(action).ok_or(RunError::UnknownAction)?;
        let Some(command) = &a.command else {
            return Err(RunError::PaneAction);
        };
        let full = format!("{plugin}:{action}");
        let req = CtxRequest::from_run_body("button", &full, body);
        let trigger = trigger_name(&req.kind);
        let ctx = self.context(&state, req).await;
        let timeout = Duration::from_secs(u64::from(
            a.timeout_secs.unwrap_or(DEFAULT_ACTION_TIMEOUT_SECS),
        ));
        let job = self.0.make_job(
            &state,
            JobKind::Action,
            format!("action:{action}"),
            full,
            command,
            timeout,
            trigger,
            ctx,
            None,
        );
        let id = job.run_id;
        self.0.submit(plugin, job);
        Ok(id)
    }

    async fn context(&self, state: &PluginState, req: CtxRequest) -> Value {
        let state_dir = self.0.paths.data(&state.entry.id);
        let version = state
            .manifest()
            .map(|m| m.version.clone())
            .unwrap_or_default();
        let facts = PluginFacts {
            id: &state.entry.id,
            version: &version,
            state_dir: &state_dir,
        };
        let ui = context::UiFacts {
            slots: self.ui_slots(),
            visible: self.ui_visible(),
        };
        context::build(self.0.lookup.as_ref(), &facts, req, &ui).await
    }

    /// Queue a run for every active plugin hook listening to `name`.
    pub async fn dispatch_event(
        &self,
        name: &str,
        data: Value,
        session: Option<String>,
        fallback: SessionInfo,
    ) {
        let states: Vec<PluginState> = self
            .0
            .state
            .lock()
            .unwrap()
            .plugins
            .iter()
            .filter(|p| p.active())
            .cloned()
            .collect();
        for state in states {
            let Some(manifest) = state.manifest().cloned() else {
                continue;
            };
            for hook in manifest
                .events
                .iter()
                .filter(|h| h.on.as_str() == name && h.on != EventName::Timer)
            {
                let mut req = CtxRequest {
                    kind: "event".into(),
                    args: json!({}),
                    session: session.clone(),
                    event: Some(
                        json!({"name": name, "at": crate::state::status::now_ms(), "data": data}),
                    ),
                    fallback: fallback.clone(),
                    workspace: data
                        .get("workspace")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| {
                            session
                                .as_deref()
                                .map(|id| crate::backend::tmux::split_session_id(id).0)
                        }),
                    ..Default::default()
                };
                req.action = Some(format!("event:{name}"));
                let ctx = self.context(&state, req).await;
                let timeout = Duration::from_secs(u64::from(
                    hook.timeout_secs.unwrap_or(DEFAULT_EVENT_TIMEOUT_SECS),
                ));
                let job = self.0.make_job(
                    &state,
                    JobKind::Event,
                    format!("event:{name}"),
                    format!("event:{name}"),
                    &hook.command,
                    timeout,
                    "event",
                    ctx,
                    None,
                );
                self.0.submit(&state.entry.id, job);
            }
        }
    }

    /// Runs queued or running for `plugin` (tests, status).
    pub fn queue_len(&self, plugin: &str) -> usize {
        self.0
            .queues
            .lock()
            .unwrap()
            .get(plugin)
            .map_or(0, |q| q.queue.len())
    }

    pub fn events_disabled(&self, plugin: &str) -> bool {
        self.0
            .queues
            .lock()
            .unwrap()
            .get(plugin)
            .is_some_and(|q| q.events_disabled)
    }

    pub fn running_count(&self, plugin: &str) -> usize {
        self.0.running.count(plugin)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunError {
    UnknownPlugin,
    UnknownAction,
    Disabled,
    /// The action opens a pane instead of running a command.
    PaneAction,
}

fn trigger_name(kind: &str) -> &'static str {
    match kind {
        "key" => "key",
        "widget" => "widget",
        "palette" => "palette",
        "event" => "event",
        "cli" => "cli",
        "timer" => "timer",
        "pane" => "pane",
        _ => "button",
    }
}

impl Inner {
    pub(crate) fn log_for(&self, plugin: &str) -> Arc<PluginLog> {
        self.logs
            .lock()
            .unwrap()
            .entry(plugin.to_owned())
            .or_insert_with(|| PluginLog::new(self.paths.log(plugin)))
            .clone()
    }

    fn notice(&self, plugin: &str, message: String) {
        self.bus.publish(DaemonEvent::PluginNotice {
            plugin: plugin.to_owned(),
            message,
            ttl_ms: None,
        });
    }

    fn resolve_dir(&self, e: &Entry) -> PathBuf {
        match &e.source {
            Source::Github { subdir, .. } => {
                layout::github_plugin_dir(&self.paths, &e.id, subdir.as_deref())
            }
            Source::Link { path } | Source::Bundled { path } => PathBuf::from(path),
        }
    }

    // ---- loading ----

    fn reload(self: &Arc<Self>, explicit: bool) -> Result<(), String> {
        let file = match PluginsFile::read(&self.paths) {
            Ok(f) => f,
            Err(e) => {
                self.state.lock().unwrap().load_error = Some(e.clone());
                return Err(e);
            }
        };
        let prev: HashMap<String, PluginState> = self
            .state
            .lock()
            .unwrap()
            .plugins
            .drain(..)
            .map(|p| (p.entry.id.clone(), p))
            .collect();
        let mut notices: Vec<(String, String)> = Vec::new();
        let mut next: Vec<PluginState> = Vec::new();
        // (id, granted permissions, manifest sha) to record under the lock.
        let mut records: Vec<(String, Option<Vec<String>>, Option<String>)> = Vec::new();
        for mut entry in file.plugins {
            let old = prev.get(&entry.id);
            let (mut state, notice) = self.load_one(&entry, old);
            // A first-party plugin whose files differ from what this build
            // shipped is not trusted, however its manifest reads.
            if entry.is_first_party()
                && state.good.is_some()
                && let Err(e) = bundled::verify_embedded(&entry.id, &state.dir)
            {
                notices.push((entry.id.clone(), e.clone()));
                state.refused = Some(e);
            }
            let (mut grant, mut sha) = (None, None);
            if let Some(good) = &state.good {
                // Approval is never automatic for code the user pointed us at:
                // only first-party (bundled) plugins whose id and manifest
                // hash are on the build's allowlist are granted at first
                // load. Anything else (a linked or GitHub plugin, a bundled
                // entry with an unknown or edited manifest) waits for
                // `plugin link` or `plugin install`, which write the grant.
                if entry.granted_permissions.is_none()
                    && entry.is_first_party()
                    && bundled::is_trusted(&entry.id, &good.sha)
                {
                    let names = good.manifest.permission_names();
                    entry.granted_permissions = Some(names.clone());
                    grant = Some(names);
                }
                if entry.granted_permissions.is_some()
                    && entry.manifest_sha256.as_deref() != Some(good.sha.as_str())
                    && state.problems.is_empty()
                {
                    entry.manifest_sha256 = Some(good.sha.clone());
                    sha = Some(good.sha.clone());
                }
                if entry.granted_permissions.is_none() {
                    state.problems.push(format!(
                        "plugin {} is not approved yet - run `claude-garage plugin link` (or install) to review and approve its permissions",
                        entry.id
                    ));
                }
            }
            if grant.is_some() || sha.is_some() {
                records.push((entry.id.clone(), grant.clone(), sha));
            }
            if let Some(n) = notice {
                notices.push((entry.id.clone(), n));
            }
            if grant.is_some() && state.token.is_empty() {
                // Approved just now: the token the load step skipped.
                match ensure_token(&self.paths, &entry.id) {
                    Ok(t) => state.token = t,
                    Err(e) => state
                        .problems
                        .push(format!("cannot create the plugin token: {e}")),
                }
            }
            state.entry = entry;
            next.push(state);
        }
        if !records.is_empty() {
            // Re-read under the registry lock: a CLI may have written since
            // our read, and its edit must survive.
            let saved = PluginsFile::update(&self.paths, |f| {
                let mut dirty = false;
                for e in f.plugins.iter_mut() {
                    let Some((_, grant, sha)) = records.iter().find(|(id, _, _)| *id == e.id)
                    else {
                        continue;
                    };
                    if let (Some(g), None) = (grant, &e.granted_permissions) {
                        e.granted_permissions = Some(g.clone());
                        dirty = true;
                    }
                    if let Some(sha) = sha
                        && e.manifest_sha256.as_ref() != Some(sha)
                    {
                        e.manifest_sha256 = Some(sha.clone());
                        dirty = true;
                    }
                }
                (dirty, ())
            });
            if let Err(e) = saved {
                eprintln!("plugins: cannot record approved permissions: {e}");
            }
        }
        // Cross-plugin cli rules, in install order.
        let conflicts = {
            let candidates: Vec<(&Manifest, bool)> = next
                .iter()
                .filter(|p| p.entry.enabled && p.entry.granted_permissions.is_some())
                .filter_map(|p| p.manifest().map(|m| (m.as_ref(), p.entry.is_first_party())))
                .collect();
            check_cli_conflicts(&candidates)
        };
        for c in conflicts {
            if let Some(p) = next.iter_mut().find(|p| p.entry.id == c.plugin) {
                p.refused = Some(c.message.clone());
                notices.push((c.plugin.clone(), c.message));
            }
        }

        // What stopped being active gets the teardown sequence.
        let was_active: Vec<&String> = prev
            .iter()
            .filter(|(_, p)| p.active())
            .map(|(id, _)| id)
            .collect();
        let still_active: Vec<&str> = next
            .iter()
            .filter(|p| p.active())
            .map(|p| p.entry.id.as_str())
            .collect();
        for id in was_active {
            if !still_active.contains(&id.as_str()) {
                self.teardown(id);
            }
        }
        // Drop widgets a reload removed or moved; restart timers and the
        // auto-disable of plugins whose manifest changed.
        for p in next.iter().filter(|p| p.active()) {
            let m = p.manifest().expect("active plugins have a manifest");
            let declared: HashMap<String, _> = m
                .widgets
                .iter()
                .map(|w| (w.id.clone(), w.slot.to_protocol()))
                .collect();
            self.widgets.retain_declared(&p.entry.id, &declared);
        }
        let signature = next
            .iter()
            .map(|p| {
                format!(
                    "{}:{}:{}:{}",
                    p.entry.id,
                    p.active(),
                    p.good.as_ref().map_or("-", |g| g.sha.as_str()),
                    p.refused.is_some()
                )
            })
            .collect::<Vec<_>>()
            .join("|");
        let changed_ids: Vec<String> = next
            .iter()
            .filter(|p| {
                explicit
                    || prev
                        .get(&p.entry.id)
                        .map(|o| (o.active(), o.good.as_ref().map(|g| g.sha.clone())))
                        != Some((p.active(), p.good.as_ref().map(|g| g.sha.clone())))
            })
            .map(|p| p.entry.id.clone())
            .collect();
        let has_links = next
            .iter()
            .any(|p| matches!(p.entry.source, Source::Link { .. }));
        let (publish, notices) = {
            let mut st = self.state.lock().unwrap();
            let publish = st.signature != signature;
            st.signature = signature;
            st.load_error = None;
            // Toasts only when the message differs from the last one shown.
            let mut fresh = Vec::new();
            for (id, msg) in notices {
                if st.notified.get(&id) != Some(&msg) {
                    st.notified.insert(id.clone(), msg.clone());
                    fresh.push((id, msg));
                }
            }
            for p in &next {
                if p.problems.is_empty() && p.refused.is_none() {
                    st.notified.remove(&p.entry.id);
                }
            }
            st.plugins = next.clone();
            (publish, fresh)
        };
        {
            let mut q = self.queues.lock().unwrap();
            for id in &changed_ids {
                if let Some(qs) = q.get_mut(id) {
                    qs.streak = 0;
                    qs.events_disabled = false;
                }
            }
        }
        for p in &next {
            if changed_ids.contains(&p.entry.id) {
                self.restart_timers(p);
            }
        }
        for (id, msg) in notices {
            self.notice(&id, msg);
        }
        if publish {
            self.bus.publish(DaemonEvent::PluginsChanged);
        }
        self.ensure_link_poller(has_links);
        Ok(())
    }

    /// Load one entry. `old` supplies the last good manifest to keep when the
    /// new one is bad or asks for more than was approved. The second value is
    /// a toast to show.
    fn load_one(&self, entry: &Entry, old: Option<&PluginState>) -> (PluginState, Option<String>) {
        let dir = self.resolve_dir(entry);
        let stamp = std::fs::metadata(dir.join(MANIFEST_FILE))
            .ok()
            .map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
        let mut state = PluginState {
            entry: entry.clone(),
            dir: dir.clone(),
            good: None,
            problems: vec![],
            refused: None,
            token: String::new(),
            seen: stamp,
        };
        let keep_old = |state: &mut PluginState| {
            if let Some(o) = old {
                state.good = o.good.clone();
            }
        };
        let raw = match std::fs::read_to_string(dir.join(MANIFEST_FILE)) {
            Ok(r) => r,
            Err(e) => {
                state.problems.push(format!(
                    "cannot read {}: {e}",
                    dir.join(MANIFEST_FILE).display()
                ));
                keep_old(&mut state);
                return self.finish_load(state, old);
            }
        };
        let manifest = match parse(&raw) {
            Ok(m) => m,
            Err(ps) => {
                state.problems = ps.iter().map(ToString::to_string).collect();
                keep_old(&mut state);
                return self.finish_load(state, old);
            }
        };
        let mut problems: Vec<String> = check_host(&manifest)
            .iter()
            .map(ToString::to_string)
            .collect();
        if manifest.id != entry.id {
            problems.push(format!(
                "id: manifest says {:?} but the registry entry is {:?}",
                manifest.id, entry.id
            ));
        }
        let wants_more = entry.granted_permissions.as_ref().is_some_and(|granted| {
            manifest
                .permission_names()
                .iter()
                .any(|p| !granted.contains(p))
        });
        if wants_more {
            problems.push(format!(
                "plugin {} wants new permissions - run `claude-garage plugin link` to approve",
                entry.id
            ));
        }
        if !problems.is_empty() {
            state.problems = problems;
            keep_old(&mut state);
            return self.finish_load(state, old);
        }
        state.good = Some(Good {
            sha: sha256_hex(raw.as_bytes()),
            manifest: Arc::new(manifest),
        });
        self.finish_load(state, old)
    }

    fn finish_load(
        &self,
        mut state: PluginState,
        old: Option<&PluginState>,
    ) -> (PluginState, Option<String>) {
        // No token until the plugin is approved: an unapproved plugin cannot
        // call the API at all.
        if state.good.is_some() && state.entry.granted_permissions.is_some() {
            match ensure_token(&self.paths, &state.entry.id) {
                Ok(t) => state.token = t,
                Err(e) => state
                    .problems
                    .push(format!("cannot create the plugin token: {e}")),
            }
        }
        let notice = if state.problems.is_empty() {
            None
        } else if state.good.is_some() && old.is_some_and(|o| o.good.is_some()) {
            // Keeping the last good manifest: say so, once.
            Some(format!(
                "{} (still running the previous version)",
                state.problems[0]
            ))
        } else {
            None
        };
        (state, notice)
    }

    fn poll_links(self: &Arc<Self>) -> bool {
        let changed = {
            let st = self.state.lock().unwrap();
            st.plugins.iter().any(|p| {
                matches!(p.entry.source, Source::Link { .. }) && {
                    let now = std::fs::metadata(p.dir.join(MANIFEST_FILE))
                        .ok()
                        .map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
                    now != p.seen
                }
            })
        };
        if changed {
            let _ = self.reload(false);
        }
        changed
    }

    fn ensure_link_poller(self: &Arc<Self>, has_links: bool) {
        let mut slot = self.link_poller.lock().unwrap();
        if !has_links {
            if let Some(h) = slot.take() {
                h.abort();
            }
            return;
        }
        if slot.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (inner, every) = (self.clone(), self.link_poll);
        *slot = Some(rt.spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                inner.poll_links();
            }
        }));
    }

    /// The disable/remove sequence (design section 4): refuse new triggers
    /// (the caller already marked it inactive), kill running process groups,
    /// drop queued runs and timers, kill its panes, clear its widgets.
    fn teardown(self: &Arc<Self>, plugin: &str) {
        if let Some(handles) = self.timers.lock().unwrap().remove(plugin) {
            handles.into_iter().for_each(|h| h.abort());
        }
        if let Some(q) = self.queues.lock().unwrap().get_mut(plugin) {
            q.queue.clear();
        }
        self.running.terminate(plugin);
        self.kill_panes_of(plugin);
        self.widgets.clear_plugin(plugin);
    }

    // ---- timers ----

    fn restart_timers(self: &Arc<Self>, p: &PluginState) {
        let id = p.entry.id.clone();
        if let Some(old) = self.timers.lock().unwrap().remove(&id) {
            old.into_iter().for_each(|h| h.abort());
        }
        let Some(manifest) = p.manifest().filter(|_| p.active()).cloned() else {
            return;
        };
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let mut handles = Vec::new();
        for (i, hook) in manifest
            .events
            .iter()
            .enumerate()
            .filter(|(_, h)| h.on == EventName::Timer)
        {
            let every = self.timer_unit * hook.every_secs.unwrap_or(5);
            let (inner, id, busy) = (self.clone(), id.clone(), Arc::new(AtomicBool::new(false)));
            handles.push(rt.spawn(async move {
                let mut iv = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
                iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    iv.tick().await;
                    inner.timer_tick(&id, i, &busy).await;
                }
            }));
        }
        if !handles.is_empty() {
            self.timers.lock().unwrap().insert(id, handles);
        }
    }

    /// One timer firing: skipped (not queued) while the previous run of the
    /// same entry is still going.
    async fn timer_tick(self: &Arc<Self>, plugin: &str, index: usize, busy: &Arc<AtomicBool>) {
        let Some(state) = self
            .state
            .lock()
            .unwrap()
            .plugins
            .iter()
            .find(|p| p.entry.id == plugin && p.active())
            .cloned()
        else {
            return;
        };
        let Some(manifest) = state.manifest().cloned() else {
            return;
        };
        let Some(hook) = manifest
            .events
            .get(index)
            .filter(|h| h.on == EventName::Timer)
        else {
            return;
        };
        if busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let guard = TimerGuard(busy.clone());
        let host = PluginHost(self.clone());
        let req = CtxRequest {
            kind: "timer".into(),
            args: json!({}),
            action: Some("timer".into()),
            event: Some(json!({"name": "timer", "at": crate::state::status::now_ms(), "data": {}})),
            ..Default::default()
        };
        let ctx = host.context(&state, req).await;
        let timeout = Duration::from_secs(u64::from(
            hook.timeout_secs.unwrap_or(DEFAULT_EVENT_TIMEOUT_SECS),
        ));
        let job = self.make_job(
            &state,
            JobKind::Timer,
            "timer".into(),
            "timer".into(),
            &hook.command,
            timeout,
            "timer",
            ctx,
            Some(guard),
        );
        self.submit(plugin, job);
    }

    // ---- the run queue ----

    #[allow(clippy::too_many_arguments)]
    fn make_job(
        &self,
        state: &PluginState,
        kind: JobKind,
        label: String,
        action: String,
        command: &[String],
        timeout: Duration,
        trigger: &'static str,
        ctx: Value,
        timer: Option<TimerGuard>,
    ) -> Job {
        let state_dir = self.paths.data(&state.entry.id);
        let platform = argv::platform_id();
        let vars = Vars {
            plugin_id: &state.entry.id,
            plugin_dir: &state.dir,
            state_dir: &state_dir,
            platform: &platform,
        };
        Job {
            run_id: self.run_seq.fetch_add(1, Ordering::Relaxed),
            kind,
            label,
            action,
            argv: argv::substitute(command, &vars),
            timeout,
            trigger,
            ctx,
            _timer: timer,
        }
    }

    fn submit(self: &Arc<Self>, plugin: &str, job: Job) {
        {
            let mut queues = self.queues.lock().unwrap();
            let q = queues.entry(plugin.to_owned()).or_default();
            if job.kind.is_hook() && q.events_disabled {
                return;
            }
            if q.queue.len() >= QUEUE_CAP
                && let Some(dropped) = q.queue.pop_front()
            {
                let (log, plugin) = (self.log_for(plugin), plugin.to_owned());
                tokio::spawn(async move {
                    log.append(
                        &format!("{} {}", dropped.run_id, dropped.label),
                        &format!("queue full for {plugin}: dropped this run"),
                    )
                    .await;
                });
            }
            q.queue.push_back(job);
        }
        self.pump(plugin);
    }

    fn pump(self: &Arc<Self>, plugin: &str) {
        loop {
            let job = {
                let mut queues = self.queues.lock().unwrap();
                let Some(q) = queues.get_mut(plugin) else {
                    return;
                };
                if q.running >= MAX_CONCURRENT {
                    return;
                }
                let Some(job) = q.queue.pop_front() else {
                    return;
                };
                q.running += 1;
                job
            };
            let (inner, plugin) = (self.clone(), plugin.to_owned());
            tokio::spawn(async move {
                let outcome = inner.execute(&plugin, &job).await;
                inner.finish(&plugin, &job, outcome);
                inner.pump(&plugin);
            });
        }
    }

    async fn execute(self: &Arc<Self>, plugin: &str, job: &Job) -> Option<RunOutcome> {
        // A plugin disabled while its run waited never starts.
        let state = self
            .state
            .lock()
            .unwrap()
            .plugins
            .iter()
            .find(|p| p.entry.id == plugin && p.active())
            .cloned()?;
        // The same check before every start: a binary swapped after the load
        // is caught here.
        if state.entry.is_first_party() {
            let (id, dir) = (plugin.to_owned(), state.dir.clone());
            let checked = tokio::task::spawn_blocking(move || bundled::verify_embedded(&id, &dir))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            if let Err(e) = checked {
                self.log_for(plugin).append("verify", &e).await;
                return Some(RunOutcome::SpawnFailed(e));
            }
        }
        let (env, ctx_file) = match self.run_env(&state, job.trigger, &job.ctx) {
            Ok(v) => v,
            Err(e) => return Some(RunOutcome::SpawnFailed(e)),
        };
        let ctx = serde_json::to_string(&job.ctx).unwrap_or_else(|_| "{}".into());
        let spec = RunSpec {
            plugin: plugin.to_owned(),
            run_id: job.run_id,
            label: job.label.clone(),
            argv: job.argv.clone(),
            cwd: state.dir.clone(),
            env,
            stdin: ctx.into_bytes(),
            timeout: Some(job.timeout),
            log: self.log_for(plugin),
            running: self.running.clone(),
        };
        let outcome = self.runner.run(spec).await;
        if let Some(path) = ctx_file {
            let _ = std::fs::remove_file(path);
        }
        Some(outcome)
    }

    /// The environment every plugin process gets (actions, hooks and panes),
    /// with the context inline or, above [`CONTEXT_ENV_MAX`], as a private
    /// file whose path is returned for the caller to remove.
    fn run_env(
        &self,
        state: &PluginState,
        trigger: &str,
        ctx_value: &Value,
    ) -> Result<RunEnv, String> {
        let plugin = &state.entry.id;
        let version = state
            .manifest()
            .map(|m| m.version.clone())
            .unwrap_or_default();
        let data_dir = self.paths.data(plugin);
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| format!("cannot create {}: {e}", data_dir.display()))?;
        let ctx = serde_json::to_string(ctx_value).unwrap_or_else(|_| "{}".into());
        let mut env = vec![
            (
                "GARAGE_API".to_owned(),
                format!("http://127.0.0.1:{}", self.port),
            ),
            ("GARAGE_API_VERSION".into(), "1".into()),
            ("GARAGE_PLUGIN_ID".into(), plugin.to_owned()),
            ("GARAGE_PLUGIN_VERSION".into(), version),
            (
                "GARAGE_PLUGIN_DIR".into(),
                state.dir.to_string_lossy().into_owned(),
            ),
            (
                "GARAGE_PLUGIN_STATE_DIR".into(),
                data_dir.to_string_lossy().into_owned(),
            ),
            ("GARAGE_PLUGIN_TOKEN".into(), state.token.clone()),
            ("GARAGE_TRIGGER".into(), trigger.to_owned()),
        ];
        let mut ctx_file: Option<PathBuf> = None;
        if ctx.len() <= CONTEXT_ENV_MAX {
            env.push(("GARAGE_CONTEXT".into(), ctx));
        } else {
            let dir = self.paths.tmp();
            let path = dir.join(format!(
                "{plugin}-{}.json",
                self.run_seq.fetch_add(1, Ordering::Relaxed)
            ));
            // The context names sessions and directories: owner-only.
            create_private_dir(&dir)
                .and_then(|_| write_private(&path, &ctx))
                .map_err(|e| format!("cannot write the context file: {e}"))?;
            env.push((
                "GARAGE_CONTEXT_FILE".into(),
                path.to_string_lossy().into_owned(),
            ));
            ctx_file = Some(path);
        }
        Ok((env, ctx_file))
    }

    /// `command` with the `{plugin_dir}`-style variables substituted.
    fn argv_for(&self, state: &PluginState, command: &[String]) -> Vec<String> {
        let state_dir = self.paths.data(&state.entry.id);
        let platform = argv::platform_id();
        let vars = Vars {
            plugin_id: &state.entry.id,
            plugin_dir: &state.dir,
            state_dir: &state_dir,
            platform: &platform,
        };
        argv::substitute(command, &vars)
    }

    fn finish(self: &Arc<Self>, plugin: &str, job: &Job, outcome: Option<RunOutcome>) {
        let mut disabled_now = false;
        {
            let mut queues = self.queues.lock().unwrap();
            let q = queues.entry(plugin.to_owned()).or_default();
            q.running = q.running.saturating_sub(1);
            if let (Some(outcome), true) = (&outcome, job.kind.is_hook()) {
                if outcome.is_success() {
                    q.streak = 0;
                } else {
                    q.streak += 1;
                    if q.streak >= FAIL_LIMIT && !q.events_disabled {
                        q.events_disabled = true;
                        disabled_now = true;
                        q.queue.retain(|j| !j.kind.is_hook());
                    }
                }
            }
        }
        let still_active = self
            .state
            .lock()
            .unwrap()
            .plugins
            .iter()
            .any(|p| p.entry.id == plugin && p.active());
        let Some(outcome) = outcome else { return };
        if !outcome.is_success() && still_active {
            self.bus.publish(DaemonEvent::PluginRunFailed {
                plugin: plugin.to_owned(),
                action: job.action.clone(),
                reason: outcome.reason(),
            });
        }
        if disabled_now {
            let msg = format!(
                "{plugin}: event hooks failed {FAIL_LIMIT} times in a row and are off until reload; see `claude-garage plugin logs {plugin}`"
            );
            let (log, line) = (self.log_for(plugin), msg.clone());
            tokio::spawn(async move { log.append("host", &line).await });
            self.notice(plugin, msg);
        }
    }
}

/// Path of a plugin's declared manifest (tests, `plugin list`).
pub fn manifest_path(dir: &Path) -> PathBuf {
    dir.join(MANIFEST_FILE)
}
