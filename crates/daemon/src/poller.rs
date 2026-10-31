//! Port of `daemon/src/poller.js`: 2 s tick joining `claude agents --json` to
//! tmux panes by pid.
//!
//! The status store, registry and statusline context store belong to other
//! lanes, so the poller talks to them through `PollerSink`; the daemon wires
//! the real stores in, tests use a fake. `claude agents --json` is behind
//! `AgentSource` so tests never run the real CLI. Session-set / title changes
//! are published as `DaemonEvent::SessionsChanged` on the bus (replacing the JS
//! `pollerEvents`).

use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use garage_protocol::SessionStatus;
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::backend::tmux::{normalize_title, split_session_id};
use crate::backend::{BackendSession, PaneInfo, SessionBackend};
use crate::bus::{DaemonEvent, EventBus};

pub const POLL_INTERVAL: Duration = Duration::from_millis(2000);

/// One entry of `claude agents --json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub pid: u32,
    /// `busy` | `waiting` | `idle`.
    pub status: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
}

impl Agent {
    /// Parse the `claude agents --json` array; anything unparseable (not an
    /// array, entries without a numeric pid) yields no agents / skips the entry.
    pub fn parse_list(json: &str) -> Vec<Agent> {
        let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json) else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|a| {
                let s = |k: &str| a.get(k).and_then(|v| v.as_str()).map(str::to_string);
                Some(Agent {
                    pid: u32::try_from(a.get("pid")?.as_u64()?).ok()?,
                    status: s("status").unwrap_or_default(),
                    session_id: s("sessionId"),
                    cwd: s("cwd"),
                })
            })
            .collect()
    }
}

/// Source of live Claude agents. Production runs `claude agents --json`; a
/// missing CLI or any failure is a silent "no agents".
#[async_trait]
pub trait AgentSource: Send + Sync {
    async fn agents(&self) -> Vec<Agent>;
}

/// Runs a command (default `claude agents --json`) and parses its stdout.
pub struct CommandAgents {
    program: String,
    args: Vec<String>,
}

impl CommandAgents {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self { program: program.into(), args }
    }
}

impl Default for CommandAgents {
    fn default() -> Self {
        Self::new("claude", vec!["agents".into(), "--json".into()])
    }
}

#[async_trait]
impl AgentSource for CommandAgents {
    async fn agents(&self) -> Vec<Agent> {
        let out = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => Agent::parse_list(&String::from_utf8_lossy(&o.stdout)),
            _ => Vec::new(),
        }
    }
}

/// The poller's view of the stores owned by other lanes.
#[async_trait]
pub trait PollerSink: Send + Sync {
    /// Current status; the "idle" default for an unknown session (JS `getStatus`).
    fn status(&self, id: &str) -> SessionStatus;
    /// Set status without a message (JS `setStatus(id, status)`).
    fn set_status(&self, id: &str, status: SessionStatus);
    /// Forget a dead session: status entry and statusline context.
    fn drop_session(&self, id: &str);
    /// Record resume metadata after a trusted pid join.
    async fn upsert_session_meta(&self, id: &str, claude_session_id: &str, workspace: &str, label: &str);
}

/// `claude agents --json` self-reports busy | waiting | idle. `waiting` is
/// blocked on the user, i.e. needs-input (2 s coarse). The poller's coarse
/// `idle` must not stomp the hook-sourced precision states: needs-input clears
/// only on real activity, and done holds until its decay or a busy tick.
pub fn apply_agent_status(sink: &dyn PollerSink, id: &str, agent_status: &str) {
    let mapped = match agent_status {
        "busy" => SessionStatus::Working,
        "waiting" => SessionStatus::NeedsInput,
        _ => SessionStatus::Idle,
    };
    let current = sink.status(id);
    if mapped == SessionStatus::Idle && matches!(current, SessionStatus::NeedsInput | SessionStatus::Done) {
        return;
    }
    sink.set_status(id, mapped);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDiff {
    pub changed: bool,
    pub ids: HashSet<String>,
    pub titles: HashMap<String, Option<String>>,
}

/// Pure id + normalized-title diff, no IO. Spawn/death of a session AND a
/// normalized-title change on a still-live one both count as changed (a
/// pane-title-only change never touches the id set).
pub fn diff_session_state(
    sessions: &[BackendSession],
    panes: &HashMap<String, PaneInfo>,
    hostname: Option<&str>,
    prev_ids: &HashSet<String>,
    prev_titles: &HashMap<String, Option<String>>,
) -> SessionDiff {
    let ids: HashSet<String> = sessions.iter().map(|s| s.id.clone()).collect();
    let mut changed = ids != *prev_ids;
    let mut titles = HashMap::new();
    for s in sessions {
        let title = normalize_title(panes.get(&s.id).map(|p| p.title.as_str()), hostname);
        // A session absent from prev_titles differs from a remembered `None`.
        if prev_titles.get(&s.id) != Some(&title) {
            changed = true;
        }
        titles.insert(s.id.clone(), title);
    }
    SessionDiff { changed, ids, titles }
}

#[derive(Default)]
struct Last {
    ids: HashSet<String>,
    titles: HashMap<String, Option<String>>,
}

pub struct Poller {
    backend: Arc<dyn SessionBackend>,
    agents: Arc<dyn AgentSource>,
    sink: Arc<dyn PollerSink>,
    bus: EventBus,
    hostname: Option<String>,
    last: Mutex<Last>,
}

impl Poller {
    pub fn new(
        backend: Arc<dyn SessionBackend>,
        agents: Arc<dyn AgentSource>,
        sink: Arc<dyn PollerSink>,
        bus: EventBus,
    ) -> Self {
        let hostname = gethostname::gethostname().to_str().map(str::to_string);
        Self { backend, agents, sink, bus, hostname, last: Mutex::new(Last::default()) }
    }

    /// Override the hostname used to filter default pane titles.
    pub fn with_hostname(mut self, hostname: Option<String>) -> Self {
        self.hostname = hostname;
        self
    }

    /// One poll. Never fails: a tick error is swallowed like the JS `pollOnce`.
    pub async fn poll_once(&self) {
        let sessions = self.backend.list().await.unwrap_or_default();

        if sessions.is_empty() {
            let had = {
                let mut last = self.last.lock().unwrap();
                let prev = std::mem::take(&mut *last);
                for id in &prev.ids {
                    self.sink.drop_session(id);
                }
                !prev.ids.is_empty()
            };
            if had {
                self.bus.publish(DaemonEvent::SessionsChanged);
            }
            return;
        }

        // One list-panes per tick daemon-wide: pid join and title diff share it.
        let (agents, panes) = tokio::join!(self.agents.agents(), async { self.backend.panes().await.unwrap_or_default() });
        let changed = {
            let mut last = self.last.lock().unwrap();
            let diff = diff_session_state(&sessions, &panes, self.hostname.as_deref(), &last.ids, &last.titles);
            for id in &last.ids {
                if !diff.ids.contains(id) {
                    self.sink.drop_session(id);
                }
            }
            last.ids = diff.ids;
            last.titles = diff.titles;
            diff.changed
        };
        if changed {
            self.bus.publish(DaemonEvent::SessionsChanged);
        }

        if agents.is_empty() {
            return; // claude CLI missing / no live agents
        }

        let by_pid: HashMap<u32, &Agent> = agents.iter().map(|a| (a.pid, a)).collect();
        let mut unmatched = Vec::new();
        for s in &sessions {
            let agent = panes.get(&s.id).and_then(|p| p.pid).and_then(|pid| by_pid.get(&pid));
            match agent {
                Some(agent) => {
                    apply_agent_status(self.sink.as_ref(), &s.id, &agent.status);
                    // Only a successful pid join yields a trusted sessionId.
                    if let Some(csid) = agent.session_id.as_deref().filter(|c| !c.is_empty()) {
                        let (workspace, label) = split_session_id(&s.id);
                        self.sink.upsert_session_meta(&s.id, csid, &workspace, &label).await;
                    }
                }
                None => unmatched.push(s),
            }
        }

        // Fail-open fallback: the pid join missed (e.g. claude exec'd through a
        // wrapper), so the first agent in the session's dir applies to ALL
        // sessions there. Over-notify rather than silently drop.
        for s in unmatched {
            if let Some(a) = agents.iter().find(|a| a.cwd.as_deref() == Some(s.dir.as_str())) {
                apply_agent_status(self.sink.as_ref(), &s.id, &a.status);
            }
        }
    }

    /// Tick every 2 s (first tick after 2 s, like `setInterval`). Abort the
    /// handle to stop.
    pub fn start(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut iv = tokio::time::interval_at(tokio::time::Instant::now() + POLL_INTERVAL, POLL_INTERVAL);
            iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                iv.tick().await;
                self.poll_once().await;
            }
        })
    }
}

#[cfg(test)]
pub(crate) mod fakes {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct FakeSink {
        pub statuses: Mutex<HashMap<String, SessionStatus>>,
        pub dropped: Mutex<Vec<String>>,
        pub metas: Mutex<Vec<(String, String, String, String)>>,
    }

    #[async_trait]
    impl PollerSink for FakeSink {
        fn status(&self, id: &str) -> SessionStatus {
            self.statuses.lock().unwrap().get(id).copied().unwrap_or(SessionStatus::Idle)
        }
        fn set_status(&self, id: &str, status: SessionStatus) {
            self.statuses.lock().unwrap().insert(id.into(), status);
        }
        fn drop_session(&self, id: &str) {
            self.statuses.lock().unwrap().remove(id);
            self.dropped.lock().unwrap().push(id.into());
        }
        async fn upsert_session_meta(&self, id: &str, c: &str, w: &str, l: &str) {
            self.metas.lock().unwrap().push((id.into(), c.into(), w.into(), l.into()));
        }
    }

    pub struct FakeBackend {
        pub sessions: Mutex<Vec<BackendSession>>,
        pub panes: Mutex<HashMap<String, PaneInfo>>,
    }

    impl FakeBackend {
        pub fn new() -> Self {
            Self { sessions: Mutex::new(vec![]), panes: Mutex::new(HashMap::new()) }
        }
        pub fn set(&self, id: &str, dir: &str, pid: Option<u32>, title: &str) {
            let mut s = self.sessions.lock().unwrap();
            s.retain(|x| x.id != id);
            s.push(BackendSession { id: id.into(), dir: dir.into(), attached: false, created_at: 1 });
            self.panes.lock().unwrap().insert(id.into(), PaneInfo { cwd: dir.into(), title: title.into(), pid });
        }
        pub fn remove(&self, id: &str) {
            self.sessions.lock().unwrap().retain(|x| x.id != id);
            self.panes.lock().unwrap().remove(id);
        }
    }

    #[async_trait]
    impl SessionBackend for FakeBackend {
        async fn list(&self) -> anyhow::Result<Vec<BackendSession>> {
            Ok(self.sessions.lock().unwrap().clone())
        }
        async fn panes(&self) -> anyhow::Result<HashMap<String, PaneInfo>> {
            Ok(self.panes.lock().unwrap().clone())
        }
        async fn has(&self, id: &str) -> anyhow::Result<bool> {
            Ok(self.sessions.lock().unwrap().iter().any(|s| s.id == id))
        }
        async fn create(&self, _: &str, _: &std::path::Path, _: &str, _: &[String], _: &[(String, String)]) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn respawn(&self, _: &str, _: &std::path::Path, _: &str, _: &[String], _: &[(String, String)]) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn kill(&self, _: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn rename(&self, _: &str, _: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn capture(&self, _: &str, _: usize) -> anyhow::Result<String> {
            unimplemented!()
        }
        fn attach_spec(&self, _: &str) -> garage_protocol::AttachSpec {
            unimplemented!()
        }
    }

    pub struct FakeAgents(pub Mutex<Vec<Agent>>);

    #[async_trait]
    impl AgentSource for FakeAgents {
        async fn agents(&self) -> Vec<Agent> {
            self.0.lock().unwrap().clone()
        }
    }

    pub fn agent(pid: u32, status: &str, sid: Option<&str>, cwd: Option<&str>) -> Agent {
        Agent { pid, status: status.into(), session_id: sid.map(Into::into), cwd: cwd.map(Into::into) }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;

    const ID: &str = "garage/test/poll";
    const HOST: &str = "my-mac";

    // ---- poller.test.js: applyAgentStatus ----
    // (The two message-retention tests -- "poller-sourced needs-input carries
    // no message" and "same-state poller tick must not clear a hook-set
    // message" -- exercise the status store and are ported by lane A.)

    fn applied(initial: Option<SessionStatus>, agent_status: &str) -> SessionStatus {
        let sink = FakeSink::default();
        if let Some(s) = initial {
            sink.set_status(ID, s);
        }
        apply_agent_status(&sink, ID, agent_status);
        sink.status(ID)
    }

    #[test]
    fn busy_maps_to_working() {
        assert_eq!(applied(None, "busy"), SessionStatus::Working);
    }

    #[test]
    fn waiting_maps_to_needs_input() {
        assert_eq!(applied(None, "waiting"), SessionStatus::NeedsInput);
    }

    #[test]
    fn idle_maps_to_idle_for_a_session_with_no_precision_state() {
        assert_eq!(applied(Some(SessionStatus::Working), "idle"), SessionStatus::Idle);
    }

    #[test]
    fn coarse_idle_must_not_stomp_hook_sourced_needs_input() {
        assert_eq!(applied(Some(SessionStatus::NeedsInput), "idle"), SessionStatus::NeedsInput);
    }

    #[test]
    fn coarse_idle_must_not_stomp_done_before_its_decay() {
        assert_eq!(applied(Some(SessionStatus::Done), "idle"), SessionStatus::Done);
    }

    #[test]
    fn busy_clears_needs_input() {
        assert_eq!(applied(Some(SessionStatus::NeedsInput), "busy"), SessionStatus::Working);
    }

    // ---- poller.test.js: diffSessionState ----

    fn session(id: &str) -> BackendSession {
        BackendSession { id: id.into(), dir: "/d".into(), attached: false, created_at: 1 }
    }
    fn pane(title: &str) -> PaneInfo {
        PaneInfo { cwd: "/d".into(), title: title.into(), pid: Some(123) }
    }
    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }
    fn titles(items: &[(&str, Option<&str>)]) -> HashMap<String, Option<String>> {
        items.iter().map(|(k, v)| (k.to_string(), v.map(str::to_string))).collect()
    }

    #[test]
    fn diff_pane_title_only_change_is_reported_changed_session_set_untouched() {
        let d = diff_session_state(
            &[session(ID)],
            &HashMap::from([(ID.into(), pane("✳ new summary"))]),
            Some(HOST),
            &set(&[ID]),
            &titles(&[(ID, Some("✳ old summary"))]),
        );
        assert!(d.changed);
        assert_eq!(d.ids, set(&[ID]));
        assert_eq!(d.titles[ID].as_deref(), Some("✳ new summary"));
    }

    #[test]
    fn diff_unchanged_id_set_and_title_reports_no_change() {
        let d = diff_session_state(
            &[session(ID)],
            &HashMap::from([(ID.into(), pane("✳ same summary"))]),
            Some(HOST),
            &set(&[ID]),
            &titles(&[(ID, Some("✳ same summary"))]),
        );
        assert!(!d.changed);
    }

    #[test]
    fn diff_title_that_normalizes_the_same_reports_no_change() {
        let d = diff_session_state(
            &[session(ID)],
            &HashMap::from([(ID.into(), pane(HOST))]),
            Some(HOST),
            &set(&[ID]),
            &titles(&[(ID, None)]),
        );
        assert!(!d.changed);
        assert_eq!(d.titles[ID], None);
    }

    #[test]
    fn diff_new_session_reports_changed_via_the_id_set() {
        let d = diff_session_state(
            &[session(ID)],
            &HashMap::from([(ID.into(), pane(""))]),
            Some(HOST),
            &set(&[]),
            &HashMap::new(),
        );
        assert!(d.changed);
        assert_eq!(d.ids, set(&[ID]));
    }

    #[test]
    fn diff_session_death_still_reports_changed() {
        let d = diff_session_state(&[], &HashMap::new(), Some(HOST), &set(&[ID]), &titles(&[(ID, Some("✳ was working"))]));
        assert!(d.changed);
        assert!(d.ids.is_empty());
        assert!(d.titles.is_empty());
    }

    #[test]
    fn diff_session_with_no_pane_entry_normalizes_to_none_without_panicking() {
        let d = diff_session_state(&[session(ID)], &HashMap::new(), Some(HOST), &set(&[ID]), &titles(&[(ID, None)]));
        assert!(!d.changed);
        assert_eq!(d.titles[ID], None);
    }

    // ---- poll_once behaviour (the JS has no direct tests; fakes, no tmux) ----

    struct Rig {
        backend: Arc<FakeBackend>,
        agents: Arc<FakeAgents>,
        sink: Arc<FakeSink>,
        poller: Poller,
        rx: tokio::sync::broadcast::Receiver<DaemonEvent>,
    }

    fn rig() -> Rig {
        let backend = Arc::new(FakeBackend::new());
        let agents = Arc::new(FakeAgents(Default::default()));
        let sink = Arc::new(FakeSink::default());
        let bus = EventBus::default();
        let rx = bus.subscribe();
        let poller = Poller::new(backend.clone(), agents.clone(), sink.clone(), bus).with_hostname(Some(HOST.into()));
        Rig { backend, agents, sink, poller, rx }
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<DaemonEvent>) -> usize {
        let mut n = 0;
        while rx.try_recv().is_ok() {
            n += 1;
        }
        n
    }

    #[tokio::test]
    async fn pid_join_sets_status_and_records_resume_meta() {
        let mut r = rig();
        r.backend.set(ID, "/d", Some(100), "");
        *r.agents.0.lock().unwrap() = vec![agent(100, "busy", Some("claude-sid"), Some("/other"))];
        r.poller.poll_once().await;
        assert_eq!(r.sink.status(ID), SessionStatus::Working);
        assert_eq!(*r.sink.metas.lock().unwrap(), [(ID.into(), "claude-sid".into(), "test".into(), "poll".into())]);
        assert_eq!(drain(&mut r.rx), 1, "a new session is one sessions-changed");
    }

    #[tokio::test]
    async fn cwd_fallback_applies_the_first_matching_agent_to_every_unmatched_session_in_that_dir() {
        let r = rig();
        r.backend.set("garage/ws/a", "/proj", Some(1), "");
        r.backend.set("garage/ws/b", "/proj", Some(2), "");
        r.backend.set("garage/ws/c", "/elsewhere", Some(3), "");
        *r.agents.0.lock().unwrap() = vec![
            agent(900, "waiting", Some("s1"), Some("/proj")),
            agent(901, "busy", None, Some("/proj")),
        ];
        r.poller.poll_once().await;
        assert_eq!(r.sink.status("garage/ws/a"), SessionStatus::NeedsInput);
        assert_eq!(r.sink.status("garage/ws/b"), SessionStatus::NeedsInput);
        assert_eq!(r.sink.status("garage/ws/c"), SessionStatus::Idle, "untouched default");
        assert!(r.sink.statuses.lock().unwrap().get("garage/ws/c").is_none());
        assert!(r.sink.metas.lock().unwrap().is_empty(), "no trusted sessionId without a pid join");
    }

    #[tokio::test]
    async fn no_agents_leaves_statuses_alone_but_still_diffs_sessions() {
        let mut r = rig();
        r.backend.set(ID, "/d", Some(1), "");
        r.poller.poll_once().await;
        assert_eq!(drain(&mut r.rx), 1);
        assert!(r.sink.statuses.lock().unwrap().is_empty());
        r.poller.poll_once().await;
        assert_eq!(drain(&mut r.rx), 0, "steady state is quiet");
    }

    #[tokio::test]
    async fn title_change_and_death_publish_sessions_changed_and_death_drops_state() {
        let mut r = rig();
        r.backend.set(ID, "/d", Some(1), "zsh");
        r.poller.poll_once().await;
        drain(&mut r.rx);

        r.backend.set(ID, "/d", Some(1), "✳ working");
        r.poller.poll_once().await;
        assert_eq!(drain(&mut r.rx), 1);

        r.backend.remove(ID);
        r.poller.poll_once().await; // empty list path
        assert_eq!(drain(&mut r.rx), 1);
        assert_eq!(*r.sink.dropped.lock().unwrap(), [ID]);
        r.poller.poll_once().await;
        assert_eq!(drain(&mut r.rx), 0, "already empty: no repeat event");
    }

    #[tokio::test]
    async fn one_of_two_sessions_dying_drops_only_that_one() {
        let mut r = rig();
        r.backend.set("garage/ws/a", "/a", Some(1), "");
        r.backend.set("garage/ws/b", "/b", Some(2), "");
        r.poller.poll_once().await;
        drain(&mut r.rx);
        r.backend.remove("garage/ws/a");
        r.poller.poll_once().await;
        assert_eq!(*r.sink.dropped.lock().unwrap(), ["garage/ws/a"]);
        assert_eq!(drain(&mut r.rx), 1);
    }

    #[test]
    fn agents_json_parsing_is_lenient() {
        let a = Agent::parse_list(r#"[{"pid":5,"status":"busy","sessionId":"x","cwd":"/c"},{"status":"busy"},{"pid":"7"}]"#);
        assert_eq!(a, [agent(5, "busy", Some("x"), Some("/c"))]);
        assert!(Agent::parse_list("not json").is_empty());
        assert!(Agent::parse_list("{}").is_empty());
    }

    #[tokio::test]
    async fn a_missing_agents_command_is_a_silent_no_op() {
        let src = CommandAgents::new("/nonexistent/claude-for-garage-tests", vec![]);
        assert!(src.agents().await.is_empty());
        let stub = CommandAgents::new("sh", vec!["-c".into(), "echo '[{\"pid\":9,\"status\":\"idle\"}]'".into()]);
        assert_eq!(stub.agents().await.len(), 1);
    }
}
