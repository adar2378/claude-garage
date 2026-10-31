//! tmux control mode (p21 section 5): one long-lived `tmux -C` client that
//! wakes the poller the moment tmux says the session set changed, instead of
//! waiting for the next tick.
//!
//! The client only ever *triggers* `Poller::poll_once`; the poll still decides
//! what changed, so bus events are exactly what the tick would have produced,
//! just sooner. `%output` is never needed (the client's own session runs
//! `cat`, so it is silent anyway). Anything that goes wrong (no server, tmux
//! exits, the client dies) clears the health flag, which puts the poller back
//! on its plain 2 s tick, and the client reconnects with backoff.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::backend::tmux::TmuxBackend;

/// Shared between the control client and the poller: a wake-up and a health
/// flag. Cheap to clone (`Arc`).
#[derive(Debug, Default)]
pub struct ControlLink {
    wake: Notify,
    healthy: AtomicBool,
}

impl ControlLink {
    /// Ask the poller for a poll soon (coalesces: many calls, one permit).
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub(crate) async fn woken(&self) {
        self.wake.notified().await;
    }

    /// Swallow a stored wake-up (used after the debounce window).
    pub(crate) fn drain(&self) {
        use futures_util::FutureExt;
        let _ = self.wake.notified().now_or_never();
    }

    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    fn set_healthy(&self, v: bool) {
        self.healthy.store(v, Ordering::Relaxed);
    }
}

/// What a control-mode line means for the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notification {
    SessionsChanged,
    SessionRenamed,
    WindowRenamed,
    Exit,
    /// Everything else: command replies, `%output`, layout, subscriptions...
    Other,
}

impl Notification {
    /// Parse one control-mode line. Only the notification name matters.
    pub fn parse(line: &str) -> Self {
        let name = line.split([' ', '\t']).next().unwrap_or("");
        match name {
            "%sessions-changed" => Self::SessionsChanged,
            "%session-renamed" => Self::SessionRenamed,
            "%window-renamed" => Self::WindowRenamed,
            "%exit" => Self::Exit,
            _ => Self::Other,
        }
    }

    pub fn wakes_poller(self) -> bool {
        self != Self::Other
    }
}

const BACKOFF_MIN: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(5);
/// A connection that lived this long resets the backoff.
const STABLE: Duration = Duration::from_secs(10);

/// Run the control client forever: connect, relay notifications, reconnect
/// with backoff. Abort the handle to stop (the tmux child is `kill_on_drop`).
pub fn spawn(backend: TmuxBackend, link: Arc<ControlLink>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = BACKOFF_MIN;
        loop {
            let started = Instant::now();
            run_once(&backend, &link).await;
            // Whatever ended the session, the tick is the source of truth
            // again; one poll right away catches what the gap hid.
            link.set_healthy(false);
            link.wake();
            if started.elapsed() >= STABLE {
                backoff = BACKOFF_MIN;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    })
}

/// One connection. Returns when the client cannot start or has ended.
async fn run_once(backend: &TmuxBackend, link: &ControlLink) {
    // Never start a server just to watch it: with no server the plain tick
    // covers everything and the next retry looks again.
    if !backend.server_running().await {
        return;
    }
    let Ok(mut child) = backend.control_command().spawn() else {
        return;
    };
    // Held open: tmux -C exits when its stdin closes.
    let _stdin = child.stdin.take();
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let mut lines = BufReader::new(stdout);
    let mut buf = Vec::new();
    let mut first = true;
    loop {
        buf.clear();
        match lines.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if first {
            // The attach reply: the client is up and subscribed to events.
            first = false;
            link.set_healthy(true);
            link.wake();
        }
        let line = String::from_utf8_lossy(&buf);
        let note = Notification::parse(line.trim_end());
        if note.wakes_poller() {
            link.wake();
        }
        if note == Notification::Exit {
            break;
        }
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_notifications_that_matter_and_ignores_the_rest() {
        assert_eq!(Notification::parse("%sessions-changed"), Notification::SessionsChanged);
        assert_eq!(Notification::parse("%session-renamed $3 garage/ws/new"), Notification::SessionRenamed);
        assert_eq!(Notification::parse("%window-renamed @1 vim"), Notification::WindowRenamed);
        assert_eq!(Notification::parse("%exit"), Notification::Exit);
        assert_eq!(Notification::parse("%exit server exited unexpectedly"), Notification::Exit);
        for other in ["%output %1 hi", "%begin 1 2 0", "%end 1 2 0", "%layout-change @1 x", "", "garbage", "%sessions-changedX"] {
            assert_eq!(Notification::parse(other), Notification::Other, "{other:?}");
            assert!(!Notification::Other.wakes_poller());
        }
    }

    // ---- real tmux, isolated server (TestTmux) ----

    use crate::backend::SessionBackend;
    use crate::bus::{DaemonEvent, EventBus};
    use crate::poller::fakes::{FakeAgents, FakeSink};
    use crate::poller::Poller;
    use crate::test_support::TestTmux;
    use tokio::sync::broadcast::Receiver;

    /// A started poller (real 2 s tick) over a real isolated tmux, plus the
    /// control client, and a bus receiver.
    struct Rig {
        tmux: TestTmux,
        backend: Arc<TmuxBackend>,
        link: Arc<ControlLink>,
        rx: Receiver<DaemonEvent>,
        tasks: Vec<JoinHandle<()>>,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            for t in &self.tasks {
                t.abort();
            }
        }
    }

    async fn rig(with_session: bool) -> Rig {
        let tmux = TestTmux::new();
        let backend = Arc::new(tmux.backend());
        if with_session {
            backend.create("garage/ws/seed", tmux.dir(), "sleep", &["60".into()], &[]).await.unwrap();
        }
        let bus = EventBus::default();
        let rx = bus.subscribe();
        let poller = Arc::new(Poller::new(
            backend.clone(),
            Arc::new(FakeAgents(Default::default())),
            Arc::new(FakeSink::default()),
            bus,
        ));
        let link = poller.control_link();
        let tasks = vec![poller.start(), spawn((*backend).clone(), link.clone())];
        Rig { tmux, backend, link, rx, tasks }
    }

    /// Next `SessionsChanged`, or `None` after `within`.
    async fn next_change(rx: &mut Receiver<DaemonEvent>, within: Duration) -> Option<Duration> {
        let t0 = Instant::now();
        let end = tokio::time::Instant::now() + within;
        loop {
            match tokio::time::timeout_at(end, rx.recv()).await {
                Ok(Ok(DaemonEvent::SessionsChanged)) => return Some(t0.elapsed()),
                Ok(Ok(_)) => continue,
                _ => return None,
            }
        }
    }

    async fn settle(r: &mut Rig) {
        // Healthy, and the connect-time catch-up poll already published.
        let link = r.link.clone();
        r.tmux.wait_for(|| link.is_healthy()).await;
        let _ = next_change(&mut r.rx, Duration::from_millis(400)).await;
        while r.rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn create_rename_and_kill_reach_the_bus_within_500ms_not_a_2s_tick_later() {
        let mut r = rig(true).await;
        settle(&mut r).await;
        let budget = Duration::from_millis(500);

        r.backend.create("garage/ws/new", r.tmux.dir(), "sleep", &["60".into()], &[]).await.unwrap();
        let create = next_change(&mut r.rx, budget).await.expect("create must wake the poller");
        tokio::time::sleep(Duration::from_millis(300)).await;
        while r.rx.try_recv().is_ok() {}

        r.backend.rename("garage/ws/new", "garage/ws/renamed").await.unwrap();
        let rename = next_change(&mut r.rx, budget).await.expect("rename must wake the poller");
        tokio::time::sleep(Duration::from_millis(300)).await;
        while r.rx.try_recv().is_ok() {}

        r.backend.kill("garage/ws/renamed").await.unwrap();
        let kill = next_change(&mut r.rx, budget).await.expect("kill must wake the poller");

        eprintln!("control-mode latency: create {create:?}, rename {rename:?}, kill {kill:?}");
    }

    #[tokio::test]
    async fn the_control_session_never_appears_as_a_garage_session() {
        let mut r = rig(true).await;
        settle(&mut r).await;
        let ids: Vec<_> = r.backend.list().await.unwrap().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["garage/ws/seed"]);
        assert!(!r.backend.panes().await.unwrap().keys().any(|k| k.contains("control")));
        // It is real and attached by exactly the control client...
        assert!(r.tmux.tmux(&["ls", "-F", "#{session_name} #{session_attached}"]).contains("garage-control 1"));
        // ...and the garage session's own attached count is untouched.
        assert!(!r.backend.list().await.unwrap()[0].attached);
        // Stopping the client leaves nothing behind.
        for t in &r.tasks {
            t.abort();
        }
        let tmux = &r.tmux;
        tmux.wait_for(|| !tmux.tmux(&["ls", "-F", "#{session_name}"]).contains("garage-control")).await;
    }

    #[tokio::test]
    async fn no_server_means_no_client_and_no_server_is_started() {
        let r = rig(false).await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(!r.link.is_healthy());
        assert_eq!(r.tmux.tmux(&["ls"]), "", "the control client must not start a server by itself");
    }

    #[tokio::test]
    async fn control_client_death_falls_back_to_the_tick_then_recovers() {
        let mut r = rig(true).await;
        settle(&mut r).await;

        // Kill the control client only (the server and sessions live on).
        r.tmux.tmux(&["detach-client", "-s", "garage-control"]);
        let link = r.link.clone();
        r.tmux.wait_for(|| !link.is_healthy()).await;
        // Unhealthy: the 2 s tick alone must still see a new session.
        r.backend.create("garage/ws/during", r.tmux.dir(), "sleep", &["60".into()], &[]).await.unwrap();
        assert!(next_change(&mut r.rx, Duration::from_secs(4)).await.is_some(), "tick covers the gap");

        // Reconnects with backoff and is fast again.
        let t0 = Instant::now();
        r.tmux.wait_for(|| link.is_healthy()).await;
        eprintln!("control-mode reconnect after client death: {:?}", t0.elapsed());
        settle(&mut r).await;
        r.backend.create("garage/ws/after", r.tmux.dir(), "sleep", &["60".into()], &[]).await.unwrap();
        assert!(next_change(&mut r.rx, Duration::from_millis(500)).await.is_some());
    }

    #[tokio::test]
    async fn server_death_then_restart_recovers_the_client() {
        let mut r = rig(true).await;
        settle(&mut r).await;
        r.tmux.tmux(&["kill-server"]);
        let link = r.link.clone();
        r.tmux.wait_for(|| !link.is_healthy()).await;
        assert!(next_change(&mut r.rx, Duration::from_secs(4)).await.is_some(), "all sessions gone is a change");

        // No busy loop while there is no server: a handful of spawns at most.
        r.backend.create("garage/ws/back", r.tmux.dir(), "sleep", &["60".into()], &[]).await.unwrap();
        r.tmux.wait_for(|| link.is_healthy()).await;
        settle(&mut r).await;
        r.backend.create("garage/ws/again", r.tmux.dir(), "sleep", &["60".into()], &[]).await.unwrap();
        assert!(next_change(&mut r.rx, Duration::from_millis(500)).await.is_some());
    }
}
