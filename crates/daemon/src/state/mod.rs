//! Daemon state: registry (state.json), status store, context store, and the
//! shared `AppState` every handler receives.

pub mod context;
pub mod registry;
pub mod status;

use std::sync::Arc;

use crate::backend::SessionBackend;
use crate::bus::EventBus;
use crate::config::Config;
use crate::os::Os;

/// Shared handler state. Cheap to clone (everything is behind an `Arc` or is
/// itself a handle). Port lanes add their stores here as fields.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub bus: EventBus,
    pub backend: Arc<dyn SessionBackend>,
    pub os: Arc<dyn Os>,
    // Later: registry / status / context stores.
}

impl AppState {
    pub fn new(config: Config, backend: Arc<dyn SessionBackend>, os: Arc<dyn Os>) -> Self {
        Self { config: Arc::new(config), bus: EventBus::default(), backend, os }
    }
}
