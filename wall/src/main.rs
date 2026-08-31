//! garage-wall entrypoint — deliberately thin (design.md: keep main small;
//! modules own the behavior).
//!
//! Order matters: the daemon health gate prints to a NORMAL terminal (before
//! raw mode / alt screen); the panic hook is installed before the terminal
//! is touched; restore runs on every exit path (the hook covers panics).

use std::process::ExitCode;

use garage_wall::api::client::{default_daemon_base_url, require_daemon};
use garage_wall::{runtime, term};

fn main() -> ExitCode {
    let base_url = default_daemon_base_url();
    // Exits(1) with an actionable `npx claude-garage` message when the
    // daemon is unreachable (spec packaging / tui-wall health gate).
    require_daemon(&base_url);

    term::install_panic_hook();
    let mut terminal = match term::enter() {
        Ok(terminal) => terminal,
        Err(e) => {
            term::restore();
            eprintln!("garage-wall: failed to initialize terminal: {e}");
            return ExitCode::FAILURE;
        }
    };

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
