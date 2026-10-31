//! Port of the launcher half of `bin/garage.js`: prerequisite checks,
//! start-or-reuse the daemon, the stale-daemon version gate, and
//! `restart [--sessions] [--all]`.
//!
//! Everything that touches the system (HTTP, spawning, signals, `lsof`,
//! prompts, sleeping, the TUI exec) goes through the [`Host`] trait, so the
//! tests drive the exact same logic against a scripted fake.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A fatal launcher error: the message goes to stderr, the process exits 1.
#[derive(Debug, PartialEq, Eq)]
pub struct Fail(pub String);

fn fail<T>(msg: impl Into<String>) -> Result<T, Fail> {
    Err(Fail(msg.into()))
}

/// stdout / stderr sinks (`console.log` / `console.error`).
pub trait Out {
    fn say(&self, line: &str);
    fn warn(&self, line: &str);
}

/// Result of running `<bin> <args>` just to see whether it exists.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Probe {
    Ok,
    /// ENOENT: the binary is not installed.
    Missing,
    /// Ran but failed (odd version-flag behaviour): treated as present.
    OtherFail,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Terminate {
    Sent,
    /// ESRCH: already gone.
    Gone,
    Error(String),
}

/// An HTTP reply: status and the body parsed as JSON (`{}` when unparseable).
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A daemon the launcher just spawned and did not wait for: the TUI waits
/// for it instead (with its startup reveal).
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Starting {
    pub log: PathBuf,
    pub pid: u32,
    pub port: u16,
}

impl Starting {
    /// The env the wall reads for the hand-off. The port is explicit so the
    /// wall waits on the daemon the launcher actually started.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("GARAGE_DAEMON_STARTING", "1".to_owned()),
            ("GARAGE_DAEMON_LOG", self.log.display().to_string()),
            ("GARAGE_DAEMON_PID", self.pid.to_string()),
            ("GARAGE_PORT", self.port.to_string()),
        ]
    }
}

pub trait Host: Out {
    fn port(&self) -> u16;
    /// The launcher's own version (compared with `/api/health`).
    fn version(&self) -> &str;
    /// Parsed `/api/health` body; `None` when no healthy daemon answers.
    /// A 200 with an unparseable body is `Some({})`.
    fn fetch_health(&self, timeout_ms: u64) -> Option<Value>;
    fn post(&self, path: &str, body: Option<&Value>) -> Result<Reply, String>;
    fn probe(&self, bin: &str, args: &[&str]) -> Probe;
    fn interactive(&self) -> bool;
    fn ask(&self, prompt: &str) -> String;
    /// Run with inherited stdio; true on exit code 0.
    fn run_inherit(&self, bin: &str, args: &[&str]) -> bool;
    fn listener_pid(&self) -> Option<u32>;
    /// Whether sessions will live in tmux (false on Windows or with
    /// `GARAGE_BACKEND=host`, where tmux is not needed).
    fn uses_tmux(&self) -> bool {
        true
    }
    fn terminate(&self, pid: u32) -> Terminate;
    /// Start `claude-garage daemon` detached; returns its log path and pid.
    fn spawn_daemon(&self) -> Result<Starting, String>;
    fn sleep_ms(&self, ms: u64);
    /// Monotonic milliseconds, for measuring real elapsed time.
    fn now_ms(&self) -> u64;
    fn resolve_tui(&self) -> Result<PathBuf, Fail>;
    /// Run the TUI in the foreground; returns its exit code (1 on a signal).
    /// `starting` is set when the daemon was just spawned: the TUI gets
    /// [`Starting::env`] and waits for the daemon itself.
    fn run_tui(&self, bin: &Path, starting: Option<&Starting>) -> Result<i32, String>;
}

/// Poll `f` every `interval_ms` until `deadline_ms` of real elapsed time
/// (probe time included, not just the sleeps) has passed.
fn wait_for<T>(
    host: &dyn Host,
    deadline_ms: u64,
    interval_ms: u64,
    mut f: impl FnMut() -> Option<T>,
) -> Option<T> {
    let start = host.now_ms();
    while host.now_ms().saturating_sub(start) < deadline_ms {
        if let Some(v) = f() {
            return Some(v);
        }
        host.sleep_ms(interval_ms);
    }
    None
}

/// Smallest/largest pid we will ever signal: JS `pid === (pid | 0) && pid > 1`.
const MIN_PID: u32 = 2;
const MAX_PID: u32 = i32::MAX as u32;

/// Convert to a signalable `pid_t`, refusing 0/1 (kill(0)/init) and anything
/// that would wrap negative (kill(-1) signals every process we may signal).
pub fn checked_pid(pid: u32) -> Result<i32, String> {
    let p = i32::try_from(pid).map_err(|_| format!("refusing to signal out-of-range pid {pid}"))?;
    if p <= 1 {
        return Err(format!("refusing to signal pid {pid}"));
    }
    Ok(p)
}

fn health_ok(host: &dyn Host, timeout_ms: u64) -> bool {
    host.fetch_health(timeout_ms).is_some()
}

fn wait_for_health(host: &dyn Host) -> bool {
    wait_for(host, 15_000, 250, || health_ok(host, 500).then_some(())).is_some()
}

fn wait_for_port_free(host: &dyn Host) -> bool {
    wait_for(host, 10_000, 250, || (!health_ok(host, 500)).then_some(())).is_some()
}

/// `Number(health.pid)`, accepted only as an integer in 2..=i32::MAX.
pub fn health_pid(health: &Value) -> Option<u32> {
    let n = match health.get("pid")? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (n.fract() == 0.0 && n >= f64::from(MIN_PID) && n <= f64::from(MAX_PID)).then_some(n as u32)
}

/// A missing `version` field (pre-upgrade daemon) counts as stale too.
pub fn daemon_is_stale(health: &Value, launcher_version: &str) -> bool {
    health.get("version").and_then(Value::as_str) != Some(launcher_version)
}

/// Parse `lsof -t` output: exactly one distinct pid greater than 1, else `None`.
pub fn parse_listener_pid(stdout: &str) -> Option<u32> {
    let mut pids: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    pids.sort_unstable();
    pids.dedup();
    if pids.len() != 1 {
        return None;
    }
    pids[0]
        .parse::<u32>()
        .ok()
        .filter(|p| (MIN_PID..=MAX_PID).contains(p))
}

// ── prerequisites ───────────────────────────────────────────────────────

fn require_binary(host: &dyn Host, bin: &str, args: &[&str], hint: &str) -> Result<(), Fail> {
    if host.probe(bin, args) == Probe::Missing {
        return fail(format!("{bin} not found — install: {hint}"));
    }
    Ok(())
}

/// tmux missing: offer a consent-gated `brew install tmux` (interactive
/// terminal and Homebrew present only), exactly like the JS launcher.
fn offer_tmux_install(host: &dyn Host) -> bool {
    if !host.interactive() {
        return false;
    }
    if host.probe("brew", &["--version"]) != Probe::Ok {
        return false;
    }
    let answer = host.ask("tmux not found. Install it now with Homebrew? [y/N] ");
    let answer = answer.trim().to_lowercase();
    if answer != "y" && answer != "yes" {
        return false;
    }
    host.say("→ brew install tmux");
    if !host.run_inherit("brew", &["install", "tmux"]) {
        return false;
    }
    host.probe("tmux", &["-V"]) == Probe::Ok
}

/// Where to get tmux on this OS.
pub fn tmux_missing_message() -> &'static str {
    if cfg!(target_os = "macos") {
        "tmux not found — install: brew install tmux"
    } else {
        "tmux not found — install it with your package manager (apt install tmux, dnf install tmux, pacman -S tmux)"
    }
}

pub fn check_prerequisites(host: &dyn Host) -> Result<(), Fail> {
    if host.uses_tmux()
        && host.probe("tmux", &["-V"]) == Probe::Missing
        && !offer_tmux_install(host)
    {
        return fail(tmux_missing_message());
    }
    require_binary(
        host,
        "claude",
        &["--version"],
        "see https://docs.claude.com/en/docs/claude-code for install instructions",
    )
}

// ── stale-daemon gate ───────────────────────────────────────────────────

fn stop_stale_daemon(host: &dyn Host, health: &Value) -> Result<(), Fail> {
    let port = host.port();
    let label = match health
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        Some(v) => format!("v{v}"),
        None => "with no version (pre-upgrade)".to_owned(),
    };
    host.say(&format!(
        "garage daemon {label} is stale (launcher v{}) — restarting it; sessions are untouched (tmux owns them)",
        host.version()
    ));
    // The pid health reports; else the exact listener on the port. Never a
    // process-name match.
    let Some(pid) = health_pid(health).or_else(|| host.listener_pid()) else {
        return fail(format!(
            "could not determine the stale daemon's pid — stop it yourself (lsof -ti tcp:{port} | xargs kill) and re-run"
        ));
    };
    match host.terminate(pid) {
        Terminate::Sent | Terminate::Gone => {} // Gone: the port check below settles it
        Terminate::Error(e) => {
            return fail(format!(
                "could not stop the stale daemon (pid {pid}): {e} — stop it yourself and re-run"
            ));
        }
    }
    if !wait_for_port_free(host) {
        return fail(format!(
            "port {port} did not free up after stopping daemon pid {pid} — something else may be serving /api/health there; stop it (lsof -ti tcp:{port}) and re-run"
        ));
    }
    Ok(())
}

// ── `claude-garage tui` ─────────────────────────────────────────────────

/// Returns the TUI's exit code.
pub fn tui_main(host: &dyn Host) -> Result<i32, Fail> {
    check_prerequisites(host)?;
    // Resolve the binary BEFORE starting a daemon: a missing binary fails
    // fast without a side-effect daemon appearing.
    let tui = host.resolve_tui()?;

    let port = host.port();
    let health = host.fetch_health(750);
    let stale = health
        .as_ref()
        .is_some_and(|h| daemon_is_stale(h, host.version()));
    if let (true, Some(h)) = (stale, &health) {
        stop_stale_daemon(host, h)?;
    }
    let mut starting = None;
    if health.is_none() || stale {
        if !stale {
            host.say(&format!(
                "no daemon on port {port} — starting one (it outlives the TUI)"
            ));
        }
        // No wait here: the TUI waits for /api/health itself, behind its
        // startup reveal, with the same 15s budget and failure message.
        starting = Some(host.spawn_daemon().map_err(Fail)?);
    }
    host.run_tui(&tui, starting.as_ref()).map_err(|e| {
        Fail(format!(
            "failed to start TUI binary at {}: {e}",
            tui.display()
        ))
    })
}

// ── `claude-garage restart` ─────────────────────────────────────────────

fn display(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "?".to_owned(),
        Some(other) => other.to_string(),
    }
}

/// Same shape as `wait_for_health`, but also needs the pid to have changed
/// and the version to match the launcher's (a not-yet-swapped daemon must
/// not read as done). `undefined` previous pid means any live match counts.
fn wait_for_health_changed(host: &dyn Host, previous: Option<&Value>) -> bool {
    wait_for(host, 15_000, 250, || {
        let h = host.fetch_health(500)?;
        (h.get("version").and_then(Value::as_str) == Some(host.version())
            && h.get("pid") != previous)
            .then_some(())
    })
    .is_some()
}

fn restart_sessions(host: &dyn Host, force: bool) -> Result<(), Fail> {
    let reply = host
        .post(
            "/api/sessions/restart",
            Some(&json!({ "all": true, "force": force })),
        )
        .map_err(|e| Fail(format!("session restart failed: {e}")))?;
    let body = &reply.body;
    if !reply.ok() {
        let msg = body
            .get("error")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        return fail(msg.map_or_else(
            || format!("session restart failed ({})", reply.status),
            str::to_owned,
        ));
    }
    let list = |k: &str| {
        body.get(k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (restarted, skipped, failed) = (list("restarted"), list("skipped"), list("failed"));
    for r in &restarted {
        let id = display(r.get("id"));
        if r.get("resumed").and_then(Value::as_bool).unwrap_or(false) {
            host.say(&format!("restarted {id} (resumed)"));
        } else {
            host.say(&format!(
                "restarted {id} (fresh — no conversation to resume)"
            ));
        }
    }
    for s in &skipped {
        host.say(&format!(
            "skipped {} — {} (use --all to include)",
            display(s.get("id")),
            display(s.get("status"))
        ));
    }
    for f in &failed {
        host.say(&format!(
            "failed {} — {}",
            display(f.get("id")),
            display(f.get("error"))
        ));
    }
    host.say(&format!(
        "{} restarted, {} skipped, {} failed",
        restarted.len(),
        skipped.len(),
        failed.len()
    ));
    Ok(())
}

pub fn restart_main(host: &dyn Host, sessions: bool, all: bool) -> Result<(), Fail> {
    let port = host.port();
    match host.fetch_health(750) {
        Some(before) => {
            let reply = host
                .post("/api/daemon/restart", None)
                .map_err(|e| Fail(format!("daemon restart request failed: {e}")))?;
            if !reply.ok() {
                return fail(format!(
                    "daemon restart request failed ({}) — the running daemon (v{}, pid {}) may predate this endpoint; stop it yourself (lsof -ti tcp:{port} | xargs kill) and re-run \"claude-garage\"",
                    reply.status,
                    display(before.get("version")),
                    display(before.get("pid")),
                ));
            }
            if !wait_for_health_changed(host, before.get("pid")) {
                return fail(format!(
                    "garage daemon did not come back up on port {port} within 15s of restarting"
                ));
            }
        }
        None => {
            host.say(&format!("no daemon on port {port} — starting one"));
            host.spawn_daemon().map_err(Fail)?;
            if !wait_for_health(host) {
                return fail(format!(
                    "claude-garage daemon did not come up on port {port}"
                ));
            }
        }
    }

    let Some(after) = host.fetch_health(750) else {
        return fail(format!(
            "garage daemon on port {port} stopped answering right after restarting"
        ));
    };
    host.say(&format!(
        "garage daemon restarted → v{} (pid {})",
        display(after.get("version")),
        display(after.get("pid"))
    ));

    if sessions {
        host.say("daemon restarted — refreshing session status before restarting sessions");
        restart_sessions(host, all)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const V: &str = "9.9.9";

    /// A scripted daemon + system. Nothing real is touched: the "daemon" is
    /// a `Value` the fake mutates when the launcher spawns / terminates /
    /// asks it to restart, and every side effect lands in `calls`.
    struct Fake {
        calls: RefCell<Vec<String>>,
        said: RefCell<Vec<String>>,
        warned: RefCell<Vec<String>>,
        daemon: RefCell<Option<Value>>,
        probes: HashMap<&'static str, Probe>,
        interactive: bool,
        answer: &'static str,
        brew_install_ok: bool,
        listener: Option<u32>,
        terminate: RefCell<Terminate>,
        terminate_frees_port: bool,
        spawn_brings_up: bool,
        restart_status: u16,
        restart_swaps: bool,
        sessions_reply: Option<Reply>,
        tui: Result<PathBuf, Fail>,
        tui_exit: i32,
        sleeps: RefCell<u64>,
        /// Fake clock cost of each health fetch (a slow probe).
        fetch_cost_ms: u64,
        fetches: RefCell<u64>,
        host_backend: bool,
    }

    impl Fake {
        fn new(daemon: Option<Value>) -> Self {
            Fake {
                calls: Default::default(),
                said: Default::default(),
                warned: Default::default(),
                daemon: RefCell::new(daemon),
                probes: HashMap::new(),
                interactive: false,
                answer: "n",
                brew_install_ok: true,
                listener: None,
                terminate: RefCell::new(Terminate::Sent),
                terminate_frees_port: true,
                spawn_brings_up: true,
                restart_status: 200,
                restart_swaps: true,
                sessions_reply: None,
                tui: Ok(PathBuf::from("/fake/garage-wall")),
                tui_exit: 0,
                sleeps: Default::default(),
                fetch_cost_ms: 0,
                fetches: Default::default(),
                host_backend: false,
            }
        }
        fn called(&self, prefix: &str) -> usize {
            self.calls
                .borrow()
                .iter()
                .filter(|c| c.starts_with(prefix))
                .count()
        }
        fn log(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        fn said(&self) -> Vec<String> {
            self.said.borrow().clone()
        }
    }

    fn health(version: Option<&str>, pid: Option<u64>) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("status".into(), json!("ok"));
        if let Some(v) = version {
            m.insert("version".into(), json!(v));
        }
        if let Some(p) = pid {
            m.insert("pid".into(), json!(p));
        }
        Value::Object(m)
    }

    impl Out for Fake {
        fn say(&self, line: &str) {
            self.said.borrow_mut().push(line.to_owned());
        }
        fn warn(&self, line: &str) {
            self.warned.borrow_mut().push(line.to_owned());
        }
    }

    impl Host for Fake {
        fn port(&self) -> u16 {
            4747
        }
        fn version(&self) -> &str {
            V
        }
        fn fetch_health(&self, _t: u64) -> Option<Value> {
            *self.fetches.borrow_mut() += 1;
            self.daemon.borrow().clone()
        }
        fn post(&self, path: &str, body: Option<&Value>) -> Result<Reply, String> {
            self.calls.borrow_mut().push(format!(
                "post {path} {}",
                body.map(Value::to_string).unwrap_or_default()
            ));
            if path == "/api/daemon/restart" {
                if self.restart_swaps && self.restart_status == 200 {
                    *self.daemon.borrow_mut() = Some(health(Some(V), Some(222)));
                }
                return Ok(Reply {
                    status: self.restart_status,
                    body: json!({}),
                });
            }
            self.sessions_reply
                .clone()
                .ok_or_else(|| "connection refused".to_owned())
        }
        fn probe(&self, bin: &str, _args: &[&str]) -> Probe {
            self.calls.borrow_mut().push(format!("probe {bin}"));
            // after a successful brew install, tmux exists
            if bin == "tmux" && self.called("run brew install tmux") > 0 && self.brew_install_ok {
                return Probe::Ok;
            }
            self.probes.get(bin).copied().unwrap_or(Probe::Ok)
        }
        fn interactive(&self) -> bool {
            self.interactive
        }
        fn ask(&self, _p: &str) -> String {
            self.calls.borrow_mut().push("ask".into());
            self.answer.to_owned()
        }
        fn run_inherit(&self, bin: &str, args: &[&str]) -> bool {
            self.calls
                .borrow_mut()
                .push(format!("run {bin} {}", args.join(" ")));
            self.brew_install_ok
        }
        fn uses_tmux(&self) -> bool {
            !self.host_backend
        }
        fn listener_pid(&self) -> Option<u32> {
            self.calls.borrow_mut().push("listener_pid".into());
            self.listener
        }
        fn terminate(&self, pid: u32) -> Terminate {
            self.calls.borrow_mut().push(format!("terminate {pid}"));
            let t = std::mem::replace(&mut *self.terminate.borrow_mut(), Terminate::Sent);
            if self.terminate_frees_port && !matches!(t, Terminate::Error(_)) {
                *self.daemon.borrow_mut() = None;
            }
            t
        }
        fn spawn_daemon(&self) -> Result<Starting, String> {
            self.calls.borrow_mut().push("spawn_daemon".into());
            if self.spawn_brings_up {
                *self.daemon.borrow_mut() = Some(health(Some(V), Some(111)));
            }
            Ok(Starting {
                log: PathBuf::from("/fake/daemon.log"),
                pid: 111,
                port: 4747,
            })
        }
        fn sleep_ms(&self, ms: u64) {
            *self.sleeps.borrow_mut() += ms;
        }
        fn now_ms(&self) -> u64 {
            *self.sleeps.borrow() + *self.fetches.borrow() * self.fetch_cost_ms
        }
        fn resolve_tui(&self) -> Result<PathBuf, Fail> {
            self.calls.borrow_mut().push("resolve_tui".into());
            match &self.tui {
                Ok(p) => Ok(p.clone()),
                Err(Fail(m)) => Err(Fail(m.clone())),
            }
        }
        fn run_tui(&self, bin: &Path, starting: Option<&Starting>) -> Result<i32, String> {
            self.calls
                .borrow_mut()
                .push(format!("run_tui {}", bin.display()));
            if let Some(s) = starting {
                let env: Vec<String> = s.env().iter().map(|(k, v)| format!("{k}={v}")).collect();
                self.calls
                    .borrow_mut()
                    .push(format!("tui_env {}", env.join(" ")));
            }
            Ok(self.tui_exit)
        }
    }

    // ── pure helpers ────────────────────────────────────────────────────

    #[test]
    fn stale_means_version_differs_or_is_missing() {
        assert!(!daemon_is_stale(&health(Some(V), Some(5)), V));
        assert!(daemon_is_stale(&health(Some("9.9.8"), Some(5)), V));
        // pre-upgrade daemons report no version at all
        assert!(daemon_is_stale(&health(None, None), V));
        // a 200 with an unparseable body arrives as {} and must read as stale,
        // not as "no daemon" (that would race a second daemon onto the port)
        assert!(daemon_is_stale(&json!({}), V));
    }

    #[test]
    fn health_pid_accepts_only_real_pids() {
        assert_eq!(health_pid(&json!({"pid": 4242})), Some(4242));
        assert_eq!(health_pid(&json!({"pid": "4242"})), Some(4242)); // Number("4242")
        for bad in [
            json!({}),
            json!({"pid": 0}),
            json!({"pid": 1}),
            json!({"pid": -5}),
            json!({"pid": 1.5}),
            json!({"pid": null}),
            json!({"pid": "abc"}),
        ] {
            assert_eq!(health_pid(&bad), None, "{bad}");
        }
    }

    #[test]
    fn pids_outside_2_to_i32_max_are_never_accepted() {
        let big = [
            json!({"pid": 2147483648u64}), // 2^31 wraps negative as pid_t
            json!({"pid": 4294967295u64}), // 2^32-1 == -1 as pid_t: kill(-1)
            json!({"pid": 4294967296u64}),
            json!({"pid": 1234.5}),
            json!({"pid": "-1"}),
            json!({"pid": "2147483648"}),
            json!({"pid": 0}),
            json!({"pid": 1}),
        ];
        for bad in big {
            assert_eq!(health_pid(&bad), None, "{bad}");
        }
        assert_eq!(health_pid(&json!({"pid": 2})), Some(2));
        assert_eq!(
            health_pid(&json!({"pid": 2147483647u64})),
            Some(i32::MAX as u32)
        );
        for bad in [
            "2147483648\n",
            "4294967295\n",
            "0\n",
            "1\n",
            "-1\n",
            "1234.5\n",
        ] {
            assert_eq!(parse_listener_pid(bad), None, "{bad:?}");
        }
        assert_eq!(parse_listener_pid("2147483647\n"), Some(i32::MAX as u32));
    }

    #[test]
    fn checked_pid_refuses_everything_that_could_signal_a_group_or_init() {
        for bad in [0u32, 1, 1 << 31, u32::MAX, u32::MAX - 1] {
            assert!(checked_pid(bad).is_err(), "{bad}");
        }
        assert_eq!(checked_pid(2), Ok(2));
        assert_eq!(checked_pid(i32::MAX as u32), Ok(i32::MAX));
    }

    #[test]
    fn the_deadline_counts_probe_time_not_just_sleeps() {
        // each health probe costs 1s of real time: a daemon that never comes
        // up must give up well before 60 sleeps (15s / 250ms) have happened.
        let mut f = Fake::new(None);
        f.spawn_brings_up = false;
        f.fetch_cost_ms = 1_000;
        assert!(restart_main(&f, false, false).is_err());
        assert!(*f.sleeps.borrow() < 15_000, "slept {}", f.sleeps.borrow());
        assert!(
            f.now_ms() >= 15_000 && f.now_ms() < 15_000 + 1_000 + 250 + 1_000,
            "{}",
            f.now_ms()
        );
    }

    #[test]
    fn listener_pid_must_be_exactly_one_distinct_pid_above_one() {
        assert_eq!(parse_listener_pid("4242\n"), Some(4242));
        assert_eq!(parse_listener_pid("4242\n4242\n"), Some(4242)); // v4 + v6 sockets, same process
        assert_eq!(parse_listener_pid("4242\n4343\n"), None); // ambiguous: never guess
        assert_eq!(parse_listener_pid(""), None);
        assert_eq!(parse_listener_pid("1\n"), None);
        assert_eq!(parse_listener_pid("junk\n"), None);
    }

    // ── prerequisites ───────────────────────────────────────────────────

    #[test]
    fn missing_tmux_without_a_terminal_fails_with_the_brew_hint_and_starts_nothing() {
        let mut f = Fake::new(None);
        f.probes.insert("tmux", Probe::Missing);
        let err = tui_main(&f).unwrap_err();
        assert_eq!(err.0, tmux_missing_message());
        assert_eq!(f.called("spawn_daemon"), 0);
        assert_eq!(f.called("ask"), 0, "never prompts off a tty");
    }

    #[test]
    fn the_host_backend_never_asks_for_tmux() {
        let mut f = Fake::new(None);
        f.probes.insert("tmux", Probe::Missing);
        f.host_backend = true;
        assert!(check_prerequisites(&f).is_ok());
        assert_eq!(f.called("probe tmux"), 0, "tmux is not even probed");
    }

    #[test]
    fn missing_tmux_on_a_tty_installs_only_after_an_explicit_yes() {
        for (answer, installs) in [
            ("y\n", true),
            (" YES \n", true),
            ("n\n", false),
            ("\n", false),
            ("yeah", false),
        ] {
            let mut f = Fake::new(None);
            f.probes.insert("tmux", Probe::Missing);
            f.probes.insert("brew", Probe::Ok);
            f.interactive = true;
            f.answer = answer;
            let r = tui_main(&f);
            assert_eq!(
                f.called("run brew install tmux"),
                usize::from(installs),
                "answer {answer:?}"
            );
            assert_eq!(r.is_ok(), installs, "answer {answer:?}");
        }
    }

    #[test]
    fn tmux_install_is_not_offered_without_homebrew_and_a_failed_install_is_fatal() {
        let mut f = Fake::new(None);
        f.probes.insert("tmux", Probe::Missing);
        f.probes.insert("brew", Probe::Missing);
        f.interactive = true;
        assert!(tui_main(&f).is_err());
        assert_eq!(f.called("ask"), 0);

        let mut f = Fake::new(None);
        f.probes.insert("tmux", Probe::Missing);
        f.interactive = true;
        f.answer = "y";
        f.brew_install_ok = false;
        assert_eq!(tui_main(&f).unwrap_err().0, tmux_missing_message());
        assert_eq!(f.called("spawn_daemon"), 0);
    }

    #[test]
    fn missing_claude_fails_but_a_binary_that_merely_errors_counts_as_present() {
        let mut f = Fake::new(None);
        f.probes.insert("claude", Probe::Missing);
        let err = tui_main(&f).unwrap_err();
        assert!(
            err.0
                .starts_with("claude not found — install: see https://docs.claude.com"),
            "{}",
            err.0
        );
        assert_eq!(f.called("spawn_daemon"), 0);

        let mut f = Fake::new(Some(health(Some(V), Some(7))));
        f.probes.insert("tmux", Probe::OtherFail);
        f.probes.insert("claude", Probe::OtherFail);
        assert_eq!(tui_main(&f), Ok(0));
    }

    // ── tui: start / reuse / gate ───────────────────────────────────────

    #[test]
    fn no_daemon_starts_one_detached_waits_for_health_then_runs_the_tui() {
        let f = Fake::new(None);
        assert_eq!(tui_main(&f), Ok(0));
        let log = f.log();
        let pos = |s: &str| {
            log.iter()
                .position(|c| c == s)
                .unwrap_or_else(|| panic!("{s} missing in {log:?}"))
        };
        assert!(
            pos("resolve_tui") < pos("spawn_daemon"),
            "binary is resolved before any daemon starts"
        );
        assert!(pos("spawn_daemon") < pos("run_tui /fake/garage-wall"));
        assert_eq!(f.called("terminate"), 0);
        assert!(f.said()[0].starts_with("no daemon on port 4747 — starting one"));
    }

    #[test]
    fn a_missing_tui_binary_fails_fast_without_a_side_effect_daemon() {
        let mut f = Fake::new(None);
        f.tui = Err(Fail("no prebuilt TUI binary".into()));
        assert_eq!(tui_main(&f).unwrap_err().0, "no prebuilt TUI binary");
        assert_eq!(f.called("spawn_daemon"), 0);
    }

    #[test]
    fn a_current_daemon_is_reused_untouched() {
        let f = Fake::new(Some(health(Some(V), Some(321))));
        assert_eq!(tui_main(&f), Ok(0));
        assert_eq!(f.called("spawn_daemon"), 0);
        assert_eq!(f.called("terminate"), 0);
        assert_eq!(f.called("listener_pid"), 0);
        assert!(f.said().is_empty());
    }

    #[test]
    fn the_tui_exit_code_is_passed_through() {
        let mut f = Fake::new(Some(health(Some(V), Some(321))));
        f.tui_exit = 3;
        assert_eq!(tui_main(&f), Ok(3));
    }

    #[test]
    fn a_stale_daemon_is_stopped_by_the_pid_it_reports_then_replaced() {
        let mut f = Fake::new(Some(health(Some("0.5.0"), Some(4242))));
        f.listener = Some(9999); // must be ignored: health reported a pid
        assert_eq!(tui_main(&f), Ok(0));
        let log = f.log();
        assert!(log.contains(&"terminate 4242".to_owned()));
        assert_eq!(f.called("terminate"), 1);
        assert_eq!(
            f.called("listener_pid"),
            0,
            "port lookup is only the fallback"
        );
        assert!(
            log.iter().position(|c| c == "terminate 4242")
                < log.iter().position(|c| c == "spawn_daemon")
        );
        let said = f.said();
        assert!(
            said[0].contains("v0.5.0 is stale (launcher v9.9.9)"),
            "{said:?}"
        );
        assert!(
            said.iter().all(|l| !l.contains("no daemon on port")),
            "stale path does not claim there was no daemon"
        );
    }

    #[test]
    fn a_pre_upgrade_daemon_without_pid_or_version_falls_back_to_the_port_listener() {
        let mut f = Fake::new(Some(health(None, None)));
        f.listener = Some(5150);
        assert_eq!(tui_main(&f), Ok(0));
        assert!(f.log().contains(&"terminate 5150".to_owned()));
        assert!(f.said()[0].contains("with no version (pre-upgrade)"));
    }

    #[test]
    fn a_reported_pid_of_one_is_never_signalled() {
        let mut f = Fake::new(Some(health(Some("0.5.0"), Some(1))));
        f.listener = Some(5150);
        assert_eq!(tui_main(&f), Ok(0));
        assert_eq!(f.called("terminate 1"), 0);
        assert!(f.log().contains(&"terminate 5150".to_owned()));
    }

    #[test]
    fn an_unknown_pid_stops_the_launch_and_signals_nobody() {
        let f = Fake::new(Some(health(None, None))); // no listener either
        let err = tui_main(&f).unwrap_err();
        assert!(
            err.0
                .starts_with("could not determine the stale daemon's pid"),
            "{}",
            err.0
        );
        assert!(err.0.contains("lsof -ti tcp:4747"));
        assert_eq!(f.called("terminate"), 0);
        assert_eq!(f.called("spawn_daemon"), 0);
    }

    #[test]
    fn an_already_gone_stale_daemon_is_not_an_error() {
        let f = Fake::new(Some(health(Some("0.5.0"), Some(4242))));
        *f.terminate.borrow_mut() = Terminate::Gone;
        assert_eq!(tui_main(&f), Ok(0));
        assert_eq!(f.called("spawn_daemon"), 1);
    }

    #[test]
    fn a_failed_signal_aborts_before_a_second_daemon_can_start() {
        let f = Fake::new(Some(health(Some("0.5.0"), Some(4242))));
        *f.terminate.borrow_mut() = Terminate::Error("Operation not permitted".into());
        let err = tui_main(&f).unwrap_err();
        assert!(
            err.0
                .contains("could not stop the stale daemon (pid 4242): Operation not permitted"),
            "{}",
            err.0
        );
        assert_eq!(f.called("spawn_daemon"), 0);
    }

    #[test]
    fn a_port_that_never_frees_aborts_after_the_10s_budget() {
        let mut f = Fake::new(Some(health(Some("0.5.0"), Some(4242))));
        f.terminate_frees_port = false;
        let err = tui_main(&f).unwrap_err();
        assert!(
            err.0
                .contains("port 4747 did not free up after stopping daemon pid 4242"),
            "{}",
            err.0
        );
        assert_eq!(f.called("spawn_daemon"), 0);
        assert_eq!(*f.sleeps.borrow(), 10_000);
    }

    #[test]
    fn a_freshly_spawned_daemon_is_not_waited_for_and_the_tui_gets_the_hand_off_env() {
        let mut f = Fake::new(None);
        // Even a daemon that never answers: the launcher no longer waits, the
        // wall owns the wait (and the failure message).
        f.spawn_brings_up = false;
        assert_eq!(tui_main(&f), Ok(0));
        assert_eq!(
            *f.sleeps.borrow(),
            0,
            "no wait_for_health on the spawn path"
        );
        assert_eq!(*f.fetches.borrow(), 1, "only the initial probe");
        assert_eq!(
            f.log().last().unwrap(),
            "tui_env GARAGE_DAEMON_STARTING=1 GARAGE_DAEMON_LOG=/fake/daemon.log GARAGE_DAEMON_PID=111 GARAGE_PORT=4747"
        );
    }

    #[test]
    fn a_daemon_that_is_already_up_gets_no_hand_off_env() {
        let f = Fake::new(Some(health(Some(V), Some(7))));
        assert_eq!(tui_main(&f), Ok(0));
        assert_eq!(f.called("tui_env"), 0);
    }

    // ── restart ─────────────────────────────────────────────────────────

    #[test]
    fn restart_asks_the_running_daemon_to_swap_and_waits_for_a_new_pid_and_matching_version() {
        let f = Fake::new(Some(health(Some(V), Some(111))));
        restart_main(&f, false, false).unwrap();
        assert_eq!(f.called("post /api/daemon/restart"), 1);
        assert_eq!(f.called("spawn_daemon"), 0);
        assert_eq!(f.called("post /api/sessions/restart"), 0);
        assert_eq!(f.said(), vec!["garage daemon restarted → v9.9.9 (pid 222)"]);
    }

    #[test]
    fn restart_does_not_accept_the_old_process_or_a_stale_version_as_done() {
        // swap never happens: same pid keeps answering
        let mut f = Fake::new(Some(health(Some(V), Some(111))));
        f.restart_swaps = false;
        let err = restart_main(&f, false, false).unwrap_err();
        assert!(
            err.0
                .contains("did not come back up on port 4747 within 15s"),
            "{}",
            err.0
        );
        assert_eq!(*f.sleeps.borrow(), 15_000);

        // new pid but an old version (mid-handoff) also does not count
        let mut f = Fake::new(Some(health(Some("0.5.0"), Some(111))));
        f.restart_swaps = false;
        *f.daemon.borrow_mut() = Some(health(Some("0.5.0"), Some(111)));
        assert!(restart_main(&f, false, false).is_err());
    }

    #[test]
    fn restart_surfaces_a_daemon_that_predates_the_endpoint() {
        let mut f = Fake::new(Some(health(Some("0.4.0"), Some(111))));
        f.restart_status = 404;
        let err = restart_main(&f, true, false).unwrap_err();
        assert!(err.0.starts_with("daemon restart request failed (404) — the running daemon (v0.4.0, pid 111) may predate this endpoint"), "{}", err.0);
        assert_eq!(
            f.called("post /api/sessions/restart"),
            0,
            "no session restarts after a failed daemon restart"
        );
    }

    #[test]
    fn restart_with_no_daemon_just_starts_one() {
        let f = Fake::new(None);
        restart_main(&f, false, false).unwrap();
        assert_eq!(f.called("post "), 0);
        assert_eq!(f.called("spawn_daemon"), 1);
        assert_eq!(f.said()[0], "no daemon on port 4747 — starting one");
        assert_eq!(f.said()[1], "garage daemon restarted → v9.9.9 (pid 111)");
    }

    #[test]
    fn restart_sessions_sends_all_true_and_force_only_with_all_and_prints_the_summary() {
        for force in [false, true] {
            let mut f = Fake::new(Some(health(Some(V), Some(111))));
            f.sessions_reply = Some(Reply {
                status: 200,
                body: json!({
                    "restarted": [{"id": "a", "resumed": true}, {"id": "b", "resumed": false}],
                    "skipped": [{"id": "c", "status": "working"}],
                    "failed": [{"id": "d", "error": "boom"}],
                }),
            });
            restart_main(&f, true, force).unwrap();
            let want = format!(
                "post /api/sessions/restart {}",
                json!({"all": true, "force": force})
            );
            assert!(f.log().contains(&want), "{:?}", f.log());
            let said = f.said();
            assert_eq!(
                said[1],
                "daemon restarted — refreshing session status before restarting sessions"
            );
            assert_eq!(
                &said[2..],
                [
                    "restarted a (resumed)",
                    "restarted b (fresh — no conversation to resume)",
                    "skipped c — working (use --all to include)",
                    "failed d — boom",
                    "2 restarted, 1 skipped, 1 failed",
                ]
            );
        }
    }

    #[test]
    fn restart_sessions_reports_the_daemons_error_body_or_the_status() {
        let mut f = Fake::new(Some(health(Some(V), Some(111))));
        f.sessions_reply = Some(Reply {
            status: 500,
            body: json!({"error": "tmux gone"}),
        });
        assert_eq!(restart_main(&f, true, false).unwrap_err().0, "tmux gone");

        let mut f = Fake::new(Some(health(Some(V), Some(111))));
        f.sessions_reply = Some(Reply {
            status: 502,
            body: json!({}),
        });
        assert_eq!(
            restart_main(&f, true, false).unwrap_err().0,
            "session restart failed (502)"
        );
    }
}
