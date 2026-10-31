//! The wall's whole startup and run sequence, callable from the single
//! `claude-garage` binary (decision 9) as well as from the thin `garage-wall`
//! binary.
//!
//! Order matters: the daemon health gate prints to a NORMAL terminal (before
//! raw mode / alt screen); the panic hook is installed before the terminal
//! is touched; restore runs on every exit path (the hook covers panics).

use std::time::Duration;

use crate::api::client::{daemon_healthy, default_daemon_base_url, require_daemon};
use crate::ui::motion::load_enabled;
use crate::ui::startup::{self, Outcome};
use crate::{runtime, term};

/// What the launcher tells the wall about the daemon it just started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Launch {
    /// Daemon base URL; `None` reads `GARAGE_PORT` like the standalone binary.
    pub base_url: Option<String>,
    /// The launcher spawned the daemon and handed over without waiting: the
    /// wall waits, behind its startup reveal (p23).
    pub daemon_starting: bool,
    /// The daemon's pid, for the "it died" check while waiting.
    pub daemon_pid: Option<u32>,
    /// The daemon's log, named in the failure messages.
    pub daemon_log: Option<String>,
}

impl Launch {
    /// The hand-off the launcher passes as `GARAGE_DAEMON_*` variables.
    pub fn from_env() -> Self {
        Self {
            base_url: None,
            daemon_starting: std::env::var_os("GARAGE_DAEMON_STARTING").is_some_and(|v| v == "1"),
            daemon_pid: startup::daemon_pid_from_env(),
            daemon_log: std::env::var("GARAGE_DAEMON_LOG").ok(),
        }
    }
}

/// Run the wall until the user quits. Never touches the daemon's or the
/// sessions' lifetime.
pub fn run(launch: Launch) -> i32 {
    let base_url = launch
        .base_url
        .clone()
        .unwrap_or_else(default_daemon_base_url);
    let daemon_pid = launch.daemon_pid;
    let launcher_starting = launch.daemon_starting;
    let log_hint = || launch.daemon_log.clone();
    let daemon_up = !launcher_starting || daemon_healthy(&base_url, Duration::from_millis(300));
    let reveal = startup::should_reveal(launcher_starting, load_enabled(), daemon_up);
    if launcher_starting && !daemon_up && !reveal {
        // Motion off: wait on the normal terminal, no animation.
        println!("starting the claude-garage daemon…");
        match startup::wait_plain(&base_url, startup::START_DEADLINE, daemon_pid) {
            Outcome::TimedOut => {
                eprintln!(
                    "{}",
                    startup::timed_out_message(&base_url, log_hint().as_deref())
                );
                return 1;
            }
            Outcome::DaemonDied => {
                eprintln!("{}", startup::daemon_died_message(log_hint().as_deref()));
                return 1;
            }
            _ => {}
        }
    }
    // Exits(1) with an actionable `npx claude-garage` message when the
    // daemon is unreachable (spec packaging / tui-wall health gate). Skipped
    // while the reveal does the waiting.
    if !reveal {
        require_daemon(&base_url);
    }

    term::install_panic_hook();
    let mut terminal = match term::enter() {
        Ok(terminal) => terminal,
        Err(e) => {
            term::restore();
            eprintln!("garage-wall: failed to initialize terminal: {e}");
            return 1;
        }
    };
    if reveal {
        match startup::reveal(
            &mut terminal,
            &base_url,
            startup::START_DEADLINE,
            daemon_pid,
        ) {
            Ok(Outcome::Ready) => {}
            Ok(Outcome::Aborted) => {
                term::restore();
                return 0;
            }
            Ok(Outcome::TimedOut) => {
                term::restore();
                eprintln!(
                    "{}",
                    startup::timed_out_message(&base_url, log_hint().as_deref())
                );
                return 1;
            }
            Ok(Outcome::DaemonDied) => {
                term::restore();
                eprintln!("{}", startup::daemon_died_message(log_hint().as_deref()));
                return 1;
            }
            Err(e) => {
                term::restore();
                eprintln!("garage-wall: startup screen failed: {e}");
                return 1;
            }
        }
    }

    let result = runtime::run(&mut terminal, &base_url);
    term::restore();

    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("garage-wall: {e}");
            1
        }
    }
}
