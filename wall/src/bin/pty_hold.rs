//! Test helper for the teardown-injection regression test
//! (`tests/injection.rs`; spike finding 2). Attaches a [`TileClient`] to
//! each named scratch tmux session, then either closes them cleanly
//! (`close` — the detach-first path) or holds them alive (`hold` — for the
//! test to SIGKILL, proving the hard-death path injects nothing either).
//!
//! Usage: `pty_hold <close|hold> <session>...`

use std::io::Write as _;
use std::time::Duration;

use garage_wall::pty::TileClient;

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let sessions: Vec<String> = args.collect();
    if sessions.is_empty() || !matches!(mode.as_str(), "close" | "hold") {
        eprintln!("usage: pty_hold <close|hold> <session>...");
        std::process::exit(2);
    }

    let mut clients: Vec<TileClient> = Vec::new();
    for session in &sessions {
        match TileClient::new(session, 80, 24, 0, || {}) {
            Ok(client) => clients.push(client),
            Err(e) => {
                eprintln!("pty_hold: attach to {session} failed: {e}");
                std::process::exit(1);
            }
        }
    }

    // Let the attach clients fully establish (alt screen, initial repaint).
    std::thread::sleep(Duration::from_millis(1500));
    println!("ATTACHED");
    let _ = std::io::stdout().flush();

    match mode.as_str() {
        "close" => {
            for client in &mut clients {
                client.close();
            }
            println!("CLOSED");
        }
        _ => {
            // "hold": stay attached until the test SIGKILLs us.
            std::thread::sleep(Duration::from_secs(120));
        }
    }
}
