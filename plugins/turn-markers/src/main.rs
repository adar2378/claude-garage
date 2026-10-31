//! `turn-markers turn | closed | started`.

use garage_plugin_sdk::{Api, read_context, state_dir};
use turn_markers::{Cmd, run};

fn main() {
    let Some(cmd) = std::env::args().nth(1).as_deref().and_then(Cmd::parse) else {
        eprintln!("turn-markers: expected one of: turn, closed, started");
        std::process::exit(2);
    };
    let result = (|| run(&Api::from_env()?, &state_dir()?, cmd, &read_context()?))();
    if let Err(e) = result {
        eprintln!("turn-markers: {e}");
        std::process::exit(1);
    }
}
