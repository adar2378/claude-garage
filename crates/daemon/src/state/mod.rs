//! Daemon state: registry (state.json), status store, context store, and the
//! shared `AppState` every handler receives.

pub mod adapters;
pub mod context;
pub mod registry;
pub mod status;

use std::path::PathBuf;
use std::sync::Arc;

use crate::backend::SessionBackend;
use crate::bus::EventBus;
use crate::config::Config;
use crate::os::Os;
use crate::poller::{AgentSource, CommandAgents, Poller};

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
            agents,
        };
        let telemetry = Telemetry::new(claude_home, Arc::new(deps));
        Self { config: Arc::new(config), bus, backend, os, registry, status, telemetry }
    }

    /// The poller (not started) over the real stores.
    pub fn poller(&self, agents: Arc<dyn AgentSource>) -> Arc<Poller> {
        let sink = adapters::StoreSink {
            registry: self.registry.clone(),
            status: self.status.clone(),
            context: self.telemetry.context.clone(),
            transcripts: self.telemetry.transcripts.clone(),
        };
        Arc::new(Poller::new(self.backend.clone(), agents, Arc::new(sink), self.bus.clone()))
    }
}
