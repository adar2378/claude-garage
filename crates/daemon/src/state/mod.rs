//! Daemon state: registry (state.json), status store, context store, and the
//! shared `AppState` every handler receives.

pub mod adapters;
pub mod context;
pub mod registry;
pub mod status;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use tokio::sync::watch;

use crate::api::daemon_restart::Lifecycle;
use crate::api::visibility::Visibility;
use crate::backend::SessionBackend;
use crate::bus::EventBus;
use crate::config::Config;
use crate::os::Os;
use crate::plugins::PluginHost;
use crate::poller::{AgentSource, CommandAgents, Poller};

pub use crate::api::sessions::ClaudeCmd;
pub use context::Telemetry;
pub use registry::Registry;
pub use status::StatusStore;

/// Shared handler state. Cheap to clone (everything is behind an `Arc` or is
/// itself a handle). Port lanes add their stores here as fields.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub bus: EventBus,
    pub backend: Arc<dyn SessionBackend>,
    pub os: Arc<dyn Os>,
    pub registry: Registry,
    pub status: StatusStore,
    /// Lane C: statusline context / rate limits, transcript cache, hook deps.
    pub telemetry: Telemetry,
    /// Lane D: the claude command sessions are started with, plus the
    /// resume-confirmation timing (injectable so tests are fast and never run
    /// the real `claude`).
    pub claude: ClaudeCmd,
    /// Shared with the poller, so `POST /api/sessions/restart` can run one
    /// poll through the same instance (and the same diff state).
    pub agents: Arc<dyn AgentSource>,
    poller_cell: Arc<OnceLock<Arc<Poller>>>,
    /// Flipped to `true` when the daemon starts shutting down, so long-lived
    /// responses (SSE) end and graceful shutdown can complete.
    shutdown: Arc<watch::Sender<bool>>,
    /// Lane F: which UI clients are in view (notification suppression).
    pub visibility: Visibility,
    /// Lane F: shutdown signal and successor command for daemon restart.
    pub lifecycle: Lifecycle,
    /// p24: the plugin host (registry, runner, widget store).
    pub plugins: PluginHost,
    /// p24: why a session is about to leave the backend (`session.closed`).
    pub closing: crate::closing::ClosingIntents,
}

impl AppState {
    /// Production wiring: `claude agents --json` and the claude home from env.
    pub fn new(config: Config, backend: Arc<dyn SessionBackend>, os: Arc<dyn Os>) -> Self {
        Self::with_agents(
            config,
            backend,
            os,
            Arc::new(CommandAgents::default()),
            crate::install::claude_home_from_env(),
        )
    }

    /// Wire the stores to each other. `agents` is shared with the poller.
    pub fn with_agents(
        config: Config,
        backend: Arc<dyn SessionBackend>,
        os: Arc<dyn Os>,
        agents: Arc<dyn AgentSource>,
        claude_home: PathBuf,
    ) -> Self {
        let bus = EventBus::default();
        let registry = Registry::new(&config.garage_dir);
        let status = StatusStore::new(bus.clone());
        let deps = adapters::StoreDeps {
            registry: registry.clone(),
            status: status.clone(),
            backend: backend.clone(),
            agents: agents.clone(),
        };
        let telemetry = Telemetry::new(claude_home, Arc::new(deps));
        let lookup = Arc::new(crate::plugins::context::StateLookup {
            backend: backend.clone(),
            status: status.clone(),
            registry: registry.clone(),
        });
        let plugins = PluginHost::new(&config, bus.clone(), lookup);
        plugins.set_backend(backend.clone());
        Self {
            config: Arc::new(config),
            bus,
            backend,
            os,
            registry,
            status,
            telemetry,
            claude: ClaudeCmd::from_env(),
            agents,
            poller_cell: Arc::default(),
            shutdown: Arc::new(watch::channel(false).0),
            visibility: Visibility::default(),
            lifecycle: Lifecycle::default(),
            plugins,
            closing: Default::default(),
        }
    }

    /// Swap the plugin runner (tests use a recorder instead of real processes).
    pub fn with_plugin_runner(mut self, runner: Arc<dyn crate::plugins::runner::Runner>) -> Self {
        let lookup = Arc::new(crate::plugins::context::StateLookup {
            backend: self.backend.clone(),
            status: self.status.clone(),
            registry: self.registry.clone(),
        });
        self.plugins = PluginHost::with_runner(&self.config, self.bus.clone(), lookup, runner);
        self.plugins.set_backend(self.backend.clone());
        self
    }

    /// Tell long-lived responses to finish; call before awaiting the server.
    pub fn begin_shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    pub fn shutdown_signal(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    /// The poller (not started) over the real stores. One instance per
    /// `AppState` (later calls return the first), so the restart route polls
    /// through the same diff state the background tick uses.
    pub fn poller(&self, agents: Arc<dyn AgentSource>) -> Arc<Poller> {
        self.poller_cell
            .get_or_init(|| self.build_poller(agents))
            .clone()
    }

    /// The shared poller over this state's own agent source.
    pub fn shared_poller(&self) -> Arc<Poller> {
        self.poller(self.agents.clone())
    }

    fn build_poller(&self, agents: Arc<dyn AgentSource>) -> Arc<Poller> {
        let sink = adapters::StoreSink {
            registry: self.registry.clone(),
            status: self.status.clone(),
            context: self.telemetry.context.clone(),
            transcripts: self.telemetry.transcripts.clone(),
        };
        Arc::new(
            Poller::new(
                self.backend.clone(),
                agents,
                Arc::new(sink),
                self.bus.clone(),
            )
            .with_intents(self.closing.clone()),
        )
    }
}
