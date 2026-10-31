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
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::backend::tmux::{normalize_title, split_session_id};
use crate::backend::{BackendSession, PaneInfo, SessionBackend, COMMAND_TIMEOUT};
use crate::bus::{DaemonEvent, EventBus};
use crate::closing::{CloseReason, ClosingIntents, Intent};
use crate::control::ControlLink;

pub const POLL_INTERVAL: Duration = Duration::from_millis(2000);
/// A notification burst (create + rename + title) becomes one poll.
pub const DEBOUNCE: Duration = Duration::from_millis(100);

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
        let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json)
        else {
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
    timeout: Duration,
}

impl CommandAgents {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            timeout: COMMAND_TIMEOUT,
        }
    }

    /// Override the command timeout (tests).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
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
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // A timeout drops the future, which kills the child: "no agents".
        match tokio::time::timeout(self.timeout, cmd.output()).await {
            Ok(Ok(o)) if o.status.success() => {
                Agent::parse_list(&String::from_utf8_lossy(&o.stdout))
            }
            _ => Vec::new(),
        }
    }
}

/// The poller's view of the stores owned by other lanes.
#[async_trait]
pub trait PollerSink: Send + Sync {
    /// Hand over a raw `claude agents --json` status (`busy` | `waiting` |
    /// `idle`). The real sink delegates to `StatusStore::apply_agent_status`,
    /// which owns the mapping and the "coarse idle must not stomp
    /// needs-input/done" rule under a single lock.
    fn apply_agent_status(&self, id: &str, agent_status: &str);
    /// Forget a dead session: status entry and statusline context.
    fn drop_session(&self, id: &str);
    /// The status name the store held for `id` (asked just before
    /// `drop_session`, for the `session.closed` event). `None` if unknown.
    fn last_status(&self, _id: &str) -> Option<String> {
        None
    }
    /// Record resume metadata after a trusted pid join.
    async fn upsert_session_meta(
        &self,
        id: &str,
        claude_session_id: &str,
        workspace: &str,
        label: &str,
    );
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
    SessionDiff {
        changed,
        ids,
        titles,
    }
}

#[derive(Default)]
struct Last {
    ids: HashSet<String>,
    titles: HashMap<String, Option<String>>,
    /// Last known directory per live id, for `session.closed`.
    dirs: HashMap<String, String>,
    /// False until the first poll: sessions that already exist when the
    /// daemon starts are not "created".
    seen_first: bool,
}

/// Membership change of one poll, published as typed events.
#[derive(Default)]
struct Membership {
    created: Vec<(String, String)>,
    closed: Vec<(String, String, Option<String>)>,
}

pub struct Poller {
    backend: Arc<dyn SessionBackend>,
    agents: Arc<dyn AgentSource>,
    sink: Arc<dyn PollerSink>,
    bus: EventBus,
    hostname: Option<String>,
    last: Mutex<Last>,
    intents: ClosingIntents,
    control: Arc<ControlLink>,
}

impl Poller {
    pub fn new(
        backend: Arc<dyn SessionBackend>,
        agents: Arc<dyn AgentSource>,
        sink: Arc<dyn PollerSink>,
        bus: EventBus,
    ) -> Self {
        let hostname = gethostname::gethostname().to_str().map(str::to_string);
        Self { backend, agents, sink, bus, hostname, last: Mutex::new(Last::default()), intents: ClosingIntents::default(), control: Arc::default() }
    }

    /// Override the hostname used to filter default pane titles.
    pub fn with_hostname(mut self, hostname: Option<String>) -> Self {
        self.hostname = hostname;
        self
    }

    /// Share the closing-intent markers the close paths write, so a vanished
    /// session gets its real `session.closed` reason.
    pub fn with_intents(mut self, intents: ClosingIntents) -> Self {
        self.intents = intents;
        self
    }

    /// Publish `session.created` / `session.closed` for one poll's membership
    /// change. A rename marker silences both halves of the rename.
    fn publish_membership(&self, m: Membership) {
        for (id, dir, last_status) in m.closed {
            let reason = match self.intents.take(&id) {
                Some(Intent::Renamed) => continue,
                Some(Intent::Reason(r)) => r,
                None => CloseReason::Crash,
            };
            let (workspace, _) = split_session_id(&id);
            self.bus.publish(DaemonEvent::SessionClosed { id, dir, workspace, reason, last_status });
        }
        for (id, dir) in m.created {
            if self.intents.take(&id) == Some(Intent::Renamed) {
                continue;
            }
            let (workspace, _) = split_session_id(&id);
            self.bus.publish(DaemonEvent::SessionCreated { id, dir, workspace });
        }
    }

    /// One poll. Never fails: a tick error is swallowed like the JS `pollOnce`.
    pub async fn poll_once(&self) {
        // A failed listing says nothing about the sessions (tmux hiccup, slow
        // socket). Treating it as "no sessions" would close everything, fire
        // hooks and write crash records for sessions that are fine: skip the
        // tick and keep what we knew.
        let Ok(sessions) = self.backend.list().await else { return };

        if sessions.is_empty() {
            let mut membership = Membership::default();
            let had = {
                let mut last = self.last.lock().unwrap();
                let prev = std::mem::take(&mut *last);
                last.seen_first = true;
                for id in &prev.ids {
                    let status = self.sink.last_status(id);
                    self.sink.drop_session(id);
                    membership.closed.push((id.clone(), prev.dirs.get(id).cloned().unwrap_or_default(), status));
                }
                !prev.ids.is_empty()
            };
            if had {
                self.publish_membership(membership);
                self.bus.publish(DaemonEvent::SessionsChanged);
            }
            return;
        }

        // One list-panes per tick daemon-wide: pid join and title diff share it.
        let (agents, panes) = tokio::join!(self.agents.agents(), async { self.backend.panes().await.unwrap_or_default() });
        let mut membership = Membership::default();
        let changed = {
            let mut last = self.last.lock().unwrap();
            let diff = diff_session_state(
                &sessions,
                &panes,
                self.hostname.as_deref(),
                &last.ids,
                &last.titles,
            );
            for id in &last.ids {
                if !diff.ids.contains(id) {
                    let status = self.sink.last_status(id);
                    self.sink.drop_session(id);
                    membership.closed.push((id.clone(), last.dirs.get(id).cloned().unwrap_or_default(), status));
                }
            }
            if last.seen_first {
                for s in &sessions {
                    if !last.ids.contains(&s.id) {
                        let dir = panes.get(&s.id).map(|p| p.cwd.clone()).unwrap_or_else(|| s.dir.clone());
                        membership.created.push((s.id.clone(), dir));
                    }
                }
            }
            last.seen_first = true;
            last.dirs = sessions
                .iter()
                .map(|s| (s.id.clone(), panes.get(&s.id).map(|p| p.cwd.clone()).unwrap_or_else(|| s.dir.clone())))
                .collect();
            last.ids = diff.ids;
            last.titles = diff.titles;
            diff.changed
        };
        self.publish_membership(membership);
        if changed {
            self.bus.publish(DaemonEvent::SessionsChanged);
        }

        if agents.is_empty() {
            return; // claude CLI missing / no live agents
        }

        let by_pid: HashMap<u32, &Agent> = agents.iter().map(|a| (a.pid, a)).collect();
        let mut unmatched = Vec::new();
        for s in &sessions {
            let agent = panes
                .get(&s.id)
                .and_then(|p| p.pid)
                .and_then(|pid| by_pid.get(&pid));
            match agent {
                Some(agent) => {
                    self.sink.apply_agent_status(&s.id, &agent.status);
                    // Only a successful pid join yields a trusted sessionId.
                    if let Some(csid) = agent.session_id.as_deref().filter(|c| !c.is_empty()) {
                        let (workspace, label) = split_session_id(&s.id);
                        self.sink
                            .upsert_session_meta(&s.id, csid, &workspace, &label)
                            .await;
                    }
                }
                None => unmatched.push(s),
            }
        }

        // Fail-open fallback: the pid join missed (e.g. claude exec'd through a
        // wrapper), so the first agent in the session's dir applies to ALL
        // sessions there. Over-notify rather than silently drop.
        for s in unmatched {
            if let Some(a) = agents
                .iter()
                .find(|a| a.cwd.as_deref() == Some(s.dir.as_str()))
            {
                self.sink.apply_agent_status(&s.id, &a.status);
            }
        }
    }

    /// The wake-up shared with the tmux control client (`control::spawn`).
    pub fn control_link(&self) -> Arc<ControlLink> {
        self.control.clone()
    }

    /// Tick every 2 s (first tick after 2 s, like `setInterval`), and also
    /// poll `DEBOUNCE` after the control client reports a change. The tick
    /// never slows down: tmux sends no notification for pane-title changes,
    /// so the tick is still what carries those. Abort the handle to stop.
    pub fn start(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut next = tokio::time::Instant::now() + POLL_INTERVAL;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(next) => {}
                    _ = self.control.woken() => {
                        tokio::time::sleep(DEBOUNCE).await;
                        // The burst is covered by the poll about to run.
                        self.control.drain();
                    }
                }
                self.poll_once().await;
                next = tokio::time::Instant::now() + POLL_INTERVAL;
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
        /// `(id, raw agent status)` in call order.
        pub applied: Mutex<Vec<(String, String)>>,
        pub dropped: Mutex<Vec<String>>,
        pub metas: Mutex<Vec<(String, String, String, String)>>,
        /// What `last_status` answers, by id.
        pub statuses: Mutex<HashMap<String, String>>,
    }

    #[async_trait]
    impl PollerSink for FakeSink {
        fn apply_agent_status(&self, id: &str, agent_status: &str) {
            self.applied
                .lock()
                .unwrap()
                .push((id.into(), agent_status.into()));
        }
        fn drop_session(&self, id: &str) {
            self.dropped.lock().unwrap().push(id.into());
        }
        fn last_status(&self, id: &str) -> Option<String> {
            self.statuses.lock().unwrap().get(id).cloned()
        }
        async fn upsert_session_meta(&self, id: &str, c: &str, w: &str, l: &str) {
            self.metas
                .lock()
                .unwrap()
                .push((id.into(), c.into(), w.into(), l.into()));
        }
    }

    pub struct FakeBackend {
        pub sessions: Mutex<Vec<BackendSession>>,
        pub panes: Mutex<HashMap<String, PaneInfo>>,
        /// While set, `list` fails like a backend that cannot be reached.
        pub fail_list: std::sync::atomic::AtomicBool,
        /// Every `create` call, in order: (id, dir, command, args, env).
        pub created: Mutex<Vec<(String, std::path::PathBuf, String, Vec<String>, Vec<(String, String)>)>>,
    }

    impl FakeBackend {
        pub fn new() -> Self {
            Self { sessions: Mutex::new(vec![]), panes: Mutex::new(HashMap::new()), fail_list: Default::default(), created: Mutex::default() }
        }
        pub fn set(&self, id: &str, dir: &str, pid: Option<u32>, title: &str) {
            let mut s = self.sessions.lock().unwrap();
            s.retain(|x| x.id != id);
            s.push(BackendSession {
                id: id.into(),
                dir: dir.into(),
                attached: false,
                created_at: Some(1),
            });
            self.panes.lock().unwrap().insert(
                id.into(),
                PaneInfo {
                    cwd: dir.into(),
                    title: title.into(),
                    pid,
                },
            );
        }
        pub fn remove(&self, id: &str) {
            self.sessions.lock().unwrap().retain(|x| x.id != id);
            self.panes.lock().unwrap().remove(id);
        }
    }

    #[async_trait]
    impl SessionBackend for FakeBackend {
        async fn init(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list(&self) -> anyhow::Result<Vec<BackendSession>> {
            if self.fail_list.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("tmux unreachable");
            }
            Ok(self.sessions.lock().unwrap().clone())
        }
        async fn panes(&self) -> anyhow::Result<HashMap<String, PaneInfo>> {
            Ok(self.panes.lock().unwrap().clone())
        }
        async fn has(&self, id: &str) -> anyhow::Result<bool> {
            Ok(self.sessions.lock().unwrap().iter().any(|s| s.id == id))
        }
        async fn create(
            &self,
            id: &str,
            dir: &std::path::Path,
            cmd: &str,
            args: &[String],
            env: &[(String, String)],
        ) -> anyhow::Result<()> {
            self.created.lock().unwrap().push((id.into(), dir.into(), cmd.into(), args.to_vec(), env.to_vec()));
            self.set(id, &dir.to_string_lossy(), None, "");
            Ok(())
        }
        async fn respawn(
            &self,
            _: &str,
            _: &std::path::Path,
            _: &str,
            _: &[String],
            _: &[(String, String)],
        ) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn kill(&self, id: &str) -> anyhow::Result<()> {
            self.remove(id);
            Ok(())
        }
        async fn plugin_pane_sessions(&self) -> anyhow::Result<Vec<String>> {
            Ok(self.sessions.lock().unwrap().iter().filter(|s| s.id.starts_with("plugin/")).map(|s| s.id.clone()).collect())
        }
        async fn rename(&self, _: &str, _: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn capture(&self, _: &str, _: usize) -> anyhow::Result<String> {
            unimplemented!()
        }
        fn attach_spec(&self, _: &str) -> garage_protocol::AttachSpec {
            garage_protocol::AttachSpec::Tmux { socket: Some("fake".into()) }
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
        Agent {
            pid,
            status: status.into(),
            session_id: sid.map(Into::into),
            cwd: cwd.map(Into::into),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;

    const ID: &str = "garage/test/poll";
    const HOST: &str = "my-mac";

    // applyAgentStatus mapping and the no-stomp rule live in StatusStore
    // (lane A); the poller only forwards the raw status.

    // ---- poller.test.js: diffSessionState ----

    fn session(id: &str) -> BackendSession {
        BackendSession {
            id: id.into(),
            dir: "/d".into(),
            attached: false,
            created_at: Some(1),
        }
    }
    fn pane(title: &str) -> PaneInfo {
        PaneInfo {
            cwd: "/d".into(),
            title: title.into(),
            pid: Some(123),
        }
    }
    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }
    fn titles(items: &[(&str, Option<&str>)]) -> HashMap<String, Option<String>> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
            .collect()
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
        let d = diff_session_state(
            &[],
            &HashMap::new(),
            Some(HOST),
            &set(&[ID]),
            &titles(&[(ID, Some("✳ was working"))]),
        );
        assert!(d.changed);
        assert!(d.ids.is_empty());
        assert!(d.titles.is_empty());
    }

    #[test]
    fn diff_session_with_no_pane_entry_normalizes_to_none_without_panicking() {
        let d = diff_session_state(
            &[session(ID)],
            &HashMap::new(),
            Some(HOST),
            &set(&[ID]),
            &titles(&[(ID, None)]),
        );
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
        let poller = Poller::new(backend.clone(), agents.clone(), sink.clone(), bus)
            .with_hostname(Some(HOST.into()));
        Rig {
            backend,
            agents,
            sink,
            poller,
            rx,
        }
    }

    /// `SessionsChanged` events only; the typed membership events have their
    /// own tests (`membership_*`).
    fn drain(rx: &mut tokio::sync::broadcast::Receiver<DaemonEvent>) -> usize {
        drain_all(rx).iter().filter(|e| **e == DaemonEvent::SessionsChanged).count()
    }

    fn drain_all(rx: &mut tokio::sync::broadcast::Receiver<DaemonEvent>) -> Vec<DaemonEvent> {
        let mut out = vec![];
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    #[tokio::test]
    async fn pid_join_sets_status_and_records_resume_meta() {
        let mut r = rig();
        r.backend.set(ID, "/d", Some(100), "");
        *r.agents.0.lock().unwrap() = vec![agent(100, "busy", Some("claude-sid"), Some("/other"))];
        r.poller.poll_once().await;
        assert_eq!(
            *r.sink.applied.lock().unwrap(),
            [(ID.to_string(), "busy".to_string())],
            "raw status forwarded"
        );
        assert_eq!(
            *r.sink.metas.lock().unwrap(),
            [(ID.into(), "claude-sid".into(), "test".into(), "poll".into())]
        );
        assert_eq!(drain(&mut r.rx), 1, "a new session is one sessions-changed");
    }

    #[tokio::test]
    async fn cwd_fallback_applies_the_first_matching_agent_to_every_unmatched_session_in_that_dir()
    {
        let r = rig();
        r.backend.set("garage/ws/a", "/proj", Some(1), "");
        r.backend.set("garage/ws/b", "/proj", Some(2), "");
        r.backend.set("garage/ws/c", "/elsewhere", Some(3), "");
        *r.agents.0.lock().unwrap() = vec![
            agent(900, "waiting", Some("s1"), Some("/proj")),
            agent(901, "busy", None, Some("/proj")),
        ];
        r.poller.poll_once().await;
        let mut applied = r.sink.applied.lock().unwrap().clone();
        applied.sort();
        assert_eq!(
            applied,
            [
                ("garage/ws/a".to_string(), "waiting".to_string()),
                ("garage/ws/b".to_string(), "waiting".to_string())
            ],
            "/elsewhere has no agent, so session c is untouched"
        );
        assert!(
            r.sink.metas.lock().unwrap().is_empty(),
            "no trusted sessionId without a pid join"
        );
    }

    #[tokio::test]
    async fn no_agents_leaves_statuses_alone_but_still_diffs_sessions() {
        let mut r = rig();
        r.backend.set(ID, "/d", Some(1), "");
        r.poller.poll_once().await;
        assert_eq!(drain(&mut r.rx), 1);
        assert!(r.sink.applied.lock().unwrap().is_empty());
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
        let a = Agent::parse_list(
            r#"[{"pid":5,"status":"busy","sessionId":"x","cwd":"/c"},{"status":"busy"},{"pid":"7"}]"#,
        );
        assert_eq!(a, [agent(5, "busy", Some("x"), Some("/c"))]);
        assert!(Agent::parse_list("not json").is_empty());
        assert!(Agent::parse_list("{}").is_empty());
    }

    #[tokio::test]
    async fn a_missing_agents_command_is_a_silent_no_op() {
        let src = CommandAgents::new("/nonexistent/claude-for-garage-tests", vec![]);
        assert!(src.agents().await.is_empty());
        let stub = CommandAgents::new(
            "sh",
            vec![
                "-c".into(),
                "echo '[{\"pid\":9,\"status\":\"idle\"}]'".into(),
            ],
        );
        assert_eq!(stub.agents().await.len(), 1);
    }

    #[tokio::test]
    async fn a_hung_agents_command_times_out_and_the_tick_still_completes() {
        let mut r = rig();
        let slow =
            CommandAgents::new("sleep", vec!["10".into()]).with_timeout(Duration::from_millis(200));
        let poller = Poller::new(
            r.backend.clone(),
            Arc::new(slow),
            r.sink.clone(),
            EventBus::default(),
        )
        .with_hostname(Some(HOST.into()));
        r.backend.set(ID, "/d", Some(1), "");
        let started = std::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(3), poller.poll_once())
            .await
            .expect("tick must not hang");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            r.sink.applied.lock().unwrap().is_empty(),
            "timeout means no agents"
        );
        drain(&mut r.rx);
    }

    // ---- p24: typed membership events ----

    fn typed(events: Vec<DaemonEvent>) -> Vec<DaemonEvent> {
        events.into_iter().filter(|e| !matches!(e, DaemonEvent::SessionsChanged)).collect()
    }

    #[tokio::test]
    async fn sessions_present_at_daemon_start_are_not_created_but_later_ones_are() {
        let mut r = rig();
        r.backend.set("garage/ws/a", "/a", Some(1), "");
        r.poller.poll_once().await;
        assert!(typed(drain_all(&mut r.rx)).is_empty(), "the first poll only learns the world");
        r.backend.set("garage/ws/b", "/b", Some(2), "");
        r.poller.poll_once().await;
        assert_eq!(
            typed(drain_all(&mut r.rx)),
            [DaemonEvent::SessionCreated { id: "garage/ws/b".into(), dir: "/b".into(), workspace: "ws".into() }]
        );
    }

    #[tokio::test]
    async fn a_vanished_session_is_a_crash_unless_a_close_marker_says_otherwise() {
        let intents = ClosingIntents::default();
        let r = rig();
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        let poller = Poller::new(r.backend.clone(), r.agents.clone(), r.sink.clone(), bus).with_intents(intents.clone());
        for id in ["garage/ws/crashed", "garage/ws/closed", "garage/ws/restarted", "garage/ws/renamed"] {
            r.backend.set(id, "/d", Some(1), "");
        }
        poller.poll_once().await;
        drain_all(&mut rx);
        intents.mark("garage/ws/closed", CloseReason::User);
        intents.mark("garage/ws/restarted", CloseReason::DaemonRestart);
        intents.mark_renamed("garage/ws/renamed");
        for id in ["garage/ws/crashed", "garage/ws/closed", "garage/ws/restarted", "garage/ws/renamed"] {
            r.backend.remove(id);
        }
        r.backend.set("garage/ws/survivor", "/s", Some(9), "");
        poller.poll_once().await;
        let mut closed: Vec<(String, &str)> = typed(drain_all(&mut rx))
            .into_iter()
            .filter_map(|e| match e {
                DaemonEvent::SessionClosed { id, reason, dir, workspace, .. } => {
                    assert_eq!((dir.as_str(), workspace.as_str()), ("/d", "ws"));
                    Some((id, reason.as_str()))
                }
                _ => None,
            })
            .collect();
        closed.sort();
        assert_eq!(
            closed,
            [
                ("garage/ws/closed".to_string(), "user"),
                ("garage/ws/crashed".to_string(), "crash"),
                ("garage/ws/restarted".to_string(), "daemon-restart"),
            ],
            "the renamed session emits neither close nor create"
        );
    }

    #[tokio::test]
    async fn a_failing_backend_listing_closes_nothing_and_remembers_what_it_knew() {
        use std::sync::atomic::Ordering;
        let mut r = rig();
        r.backend.set("garage/ws/a", "/a", Some(1), "");
        r.backend.set("garage/ws/b", "/b", Some(2), "");
        r.poller.poll_once().await;
        drain_all(&mut r.rx);
        r.backend.fail_list.store(true, Ordering::SeqCst);
        for _ in 0..3 {
            r.poller.poll_once().await;
        }
        assert!(typed(drain_all(&mut r.rx)).is_empty(), "no close, no create, no sessions-changed");
        assert!(r.sink.dropped.lock().unwrap().is_empty(), "statuses are not dropped");
        // tmux comes back with the same sessions: nothing changed
        r.backend.fail_list.store(false, Ordering::SeqCst);
        r.poller.poll_once().await;
        assert!(typed(drain_all(&mut r.rx)).is_empty(), "the memory survived the outage");
        // and a real vanish afterwards is still seen, exactly once
        r.backend.remove("garage/ws/b");
        r.poller.poll_once().await;
        let closed: Vec<_> = typed(drain_all(&mut r.rx)).into_iter().filter(|e| matches!(e, DaemonEvent::SessionClosed { .. })).collect();
        assert_eq!(closed.len(), 1);
    }

    #[tokio::test]
    async fn the_last_session_vanishing_still_reports_its_close_with_the_last_known_status() {
        let mut r = rig();
        r.backend.set(ID, "/work", Some(1), "");
        r.sink.statuses.lock().unwrap().insert(ID.into(), "working".into());
        r.poller.poll_once().await;
        drain_all(&mut r.rx);
        r.backend.remove(ID);
        r.poller.poll_once().await; // the empty-list path
        let ev = typed(drain_all(&mut r.rx));
        assert_eq!(ev.len(), 1, "{ev:?}");
        assert!(matches!(&ev[0], DaemonEvent::SessionClosed { id, dir, reason: CloseReason::Crash, last_status: Some(s), .. }
            if id == ID && dir == "/work" && s == "working"), "{ev:?}");
    }

    // ---- start(): tick + control wake-ups (paused clock, fakes) ----

    /// Counts polls: `agents()` runs once per poll that has live sessions.
    struct CountingAgents(std::sync::atomic::AtomicUsize);

    #[async_trait]
    impl AgentSource for CountingAgents {
        async fn agents(&self) -> Vec<Agent> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Vec::new()
        }
    }

    impl CountingAgents {
        fn polls(&self) -> usize {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    async fn advance(ms: u64) {
        tokio::time::advance(Duration::from_millis(ms)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_wakeups_is_one_debounced_poll_and_the_tick_still_runs() {
        let backend = Arc::new(FakeBackend::new());
        backend.set(ID, "/d", Some(1), "");
        let agents = Arc::new(CountingAgents(Default::default()));
        let poller = Arc::new(Poller::new(
            backend.clone(),
            agents.clone(),
            Arc::new(FakeSink::default()),
            EventBus::default(),
        ));
        let link = poller.control_link();
        let task = poller.start();
        advance(1).await;
        assert_eq!(
            agents.polls(),
            0,
            "nothing before the first tick or a wake-up"
        );

        for _ in 0..5 {
            link.wake();
            advance(10).await;
        }
        assert_eq!(agents.polls(), 0, "still inside the debounce window");
        advance(150).await;
        assert_eq!(
            agents.polls(),
            1,
            "five wake-ups, one poll, ~100 ms after the first"
        );
        advance(1500).await;
        assert_eq!(agents.polls(), 1, "no stray poll from the stored permit");

        // The tick is the fallback and fires 2 s after the last poll.
        advance(600).await;
        assert_eq!(agents.polls(), 2);
        task.abort();
    }
}
