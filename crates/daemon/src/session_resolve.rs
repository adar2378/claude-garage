//! Port of `daemon/src/session-resolve.js`: map a Claude Code hook / statusline
//! payload (`session_id`, `cwd`) to the garage session id(s) it applies to.
//!
//! Precise path: payload `session_id` -> `claude agents --json` entry with that
//! `sessionId` -> its pid -> the tmux pane with that root pid. Fallback: `cwd`
//! fail-open, applied to every garage session sharing that dir (over-notify
//! rather than silently drop).

use crate::backend::SessionBackend;
use crate::poller::AgentSource;

pub async fn resolve_session_ids(
    backend: &dyn SessionBackend,
    agents: &dyn AgentSource,
    session_id: Option<&str>,
    cwd: Option<&str>,
) -> Vec<String> {
    if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
        let (agents, panes) = tokio::join!(agents.agents(), async {
            backend.panes().await.unwrap_or_default()
        });
        let name = agents
            .iter()
            .find(|a| a.session_id.as_deref() == Some(sid))
            .and_then(|a| {
                panes
                    .iter()
                    .find(|(_, p)| p.pid == Some(a.pid))
                    .map(|(name, _)| name.clone())
            });
        if let Some(name) = name {
            return vec![name];
        }
    }

    if let Some(cwd) = cwd.filter(|c| !c.is_empty()) {
        let sessions = backend.list().await.unwrap_or_default();
        let matches: Vec<String> = sessions
            .into_iter()
            .filter(|s| s.dir == cwd)
            .map(|s| s.id)
            .collect();
        if !matches.is_empty() {
            return matches;
        }
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poller::fakes::{FakeAgents, FakeBackend, agent};

    fn rig() -> (FakeBackend, FakeAgents) {
        let b = FakeBackend::new();
        b.set("garage/ws/a", "/proj", Some(10), "");
        b.set("garage/ws/b", "/proj", Some(20), "");
        b.set("garage/ws/c", "/solo", Some(30), "");
        let a = FakeAgents(Default::default());
        *a.0.lock().unwrap() = vec![agent(20, "busy", Some("sid-b"), Some("/proj"))];
        (b, a)
    }

    #[tokio::test]
    async fn session_id_resolves_precisely_through_the_pid_join() {
        let (b, a) = rig();
        assert_eq!(
            resolve_session_ids(&b, &a, Some("sid-b"), Some("/proj")).await,
            ["garage/ws/b"]
        );
    }

    #[tokio::test]
    async fn unknown_session_id_falls_back_to_every_session_in_the_cwd() {
        let (b, a) = rig();
        let mut got = resolve_session_ids(&b, &a, Some("nope"), Some("/proj")).await;
        got.sort();
        assert_eq!(got, ["garage/ws/a", "garage/ws/b"]);
        assert_eq!(
            resolve_session_ids(&b, &a, None, Some("/solo")).await,
            ["garage/ws/c"]
        );
    }

    #[tokio::test]
    async fn agent_whose_pid_matches_no_pane_falls_back_to_cwd() {
        let (b, a) = rig();
        *a.0.lock().unwrap() = vec![agent(999, "busy", Some("sid-x"), None)];
        assert_eq!(
            resolve_session_ids(&b, &a, Some("sid-x"), Some("/solo")).await,
            ["garage/ws/c"]
        );
    }

    #[tokio::test]
    async fn nothing_matches_gives_an_empty_list() {
        let (b, a) = rig();
        assert!(
            resolve_session_ids(&b, &a, Some("nope"), Some("/nowhere"))
                .await
                .is_empty()
        );
        assert!(resolve_session_ids(&b, &a, None, None).await.is_empty());
        assert!(
            resolve_session_ids(&b, &a, Some(""), Some(""))
                .await
                .is_empty()
        );
    }
}
