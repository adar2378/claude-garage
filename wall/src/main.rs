//! garage-wall entrypoint — deliberately thin (design.md: keep main small;
//! modules own the behavior).
//!
//! Order matters: the daemon health gate prints to a NORMAL terminal (before
//! raw mode / alt screen); the panic hook is installed before the terminal
//! is touched; restore runs on every exit path (the hook covers panics).

use std::process::ExitCode;

use std::time::Duration;

use garage_wall::api::client::{daemon_healthy, default_daemon_base_url, require_daemon};
use garage_wall::ui::motion::load_enabled;
use garage_wall::ui::startup::{self, Outcome};
use garage_wall::{runtime, term};

fn log_hint() -> Option<String> {
    std::env::var("GARAGE_DAEMON_LOG").ok()
}

fn main() -> ExitCode {
    let base_url = default_daemon_base_url();
    // The launcher sets this when it just started the daemon and handed the
    // terminal over without waiting (p23): the wall waits, with the reveal.
    let daemon_pid = startup::daemon_pid_from_env();
    let launcher_starting = std::env::var_os("GARAGE_DAEMON_STARTING").is_some_and(|v| v == "1");
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
                return ExitCode::FAILURE;
            }
            Outcome::DaemonDied => {
                eprintln!("{}", startup::daemon_died_message(log_hint().as_deref()));
                return ExitCode::FAILURE;
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
            return ExitCode::FAILURE;
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
                return ExitCode::SUCCESS;
            }
            Ok(Outcome::TimedOut) => {
                term::restore();
                eprintln!(
                    "{}",
                    startup::timed_out_message(&base_url, log_hint().as_deref())
                );
                return ExitCode::FAILURE;
            }
            Ok(Outcome::DaemonDied) => {
                term::restore();
                eprintln!("{}", startup::daemon_died_message(log_hint().as_deref()));
                return ExitCode::FAILURE;
            }
            Err(e) => {
                term::restore();
                eprintln!("garage-wall: startup screen failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    let result = runtime::run(&mut terminal, &base_url);
    term::restore();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("garage-wall: {e}");
            ExitCode::FAILURE
        }
    }
}
