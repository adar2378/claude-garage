//! Production adapters joining the lanes: the poller's sink and the telemetry
//! deps, both over the real stores.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use garage_protocol::SessionStatus;

use super::context::{ContextStore, TelemetryDeps};
use super::registry::{MetaUpdate, Registry};
use super::status::StatusStore;
use crate::backend::SessionBackend;
use crate::poller::{AgentSource, PollerSink};
use crate::session_resolve::resolve_session_ids;
use crate::transcript::TranscriptCache;

/// `PollerSink` over the status store, registry, statusline context and
/// transcript cache.
pub struct StoreSink {
    pub registry: Registry,
    pub status: StatusStore,
    pub context: ContextStore,
    pub transcripts: TranscriptCache,
}

#[async_trait]
impl PollerSink for StoreSink {
    fn apply_agent_status(&self, id: &str, agent_status: &str) {
        // Owns the mapping and the "coarse idle must not stomp" guard.
        self.status.apply_agent_status(id, agent_status);
    }

    fn drop_session(&self, id: &str) {
        self.status.drop_session(id);
        self.context.drop_session(id);
        self.transcripts.clear_cache(id);
    }

    async fn upsert_session_meta(
        &self,
        id: &str,
        claude_session_id: &str,
        workspace: &str,
        label: &str,
    ) {
        let upd = MetaUpdate {
            claude_session_id: Some(Some(claude_session_id.to_owned())),
            workspace: Some(workspace.to_owned()),
            label: Some(label.to_owned()),
            worktree: None,
        };
        // Best effort, like the JS poller: a failed write is retried next tick.
        let _ = self.registry.upsert_session_meta(id, upd).await;
    }
}

/// `TelemetryDeps` over the registry (hook token), status store and the
/// session-resolve join (backend panes x `claude agents`).
pub struct StoreDeps {
    pub registry: Registry,
    pub status: StatusStore,
    pub backend: Arc<dyn SessionBackend>,
    pub agents: Arc<dyn AgentSource>,
}

#[async_trait]
impl TelemetryDeps for StoreDeps {
    async fn hook_token(&self) -> Result<String> {
        self.registry
            .get_hook_token()
            .await
            .context("reading the hook token")
    }

    fn set_status(&self, id: &str, state: SessionStatus, message: Option<Option<String>>) {
        // `StatusStore::set` takes one optional message: undefined and null
        // both mean "no new message" (it keeps or clears per its own rules).
        self.status.set(id, state, message.flatten().as_deref());
    }

    async fn resolve_session_ids(
        &self,
        session_id: Option<&str>,
        cwd: Option<&str>,
    ) -> Vec<String> {
        resolve_session_ids(self.backend.as_ref(), self.agents.as_ref(), session_id, cwd).await
    }
}
