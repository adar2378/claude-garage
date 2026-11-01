//! `alerts status | cli`: see the library for what they do.

use alerts::{RealClock, SystemOut};
use garage_plugin_sdk::{Api, Context, state_dir};

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("status") => {
            // Always exit 0: five failed runs in a row switch the hook off,
            // and a missing sound player is no reason for that.
            let result = (|| {
                let ctx = Context::read()?;
                let api = Api::from_env()?;
                let dir = state_dir()?;
                alerts::handle_event(&ctx, &api, &dir, &SystemOut, &RealClock)
            })();
            if let Err(e) = result {
                eprintln!("alerts: {e}");
            }
        }
        Some("cli") => {
            let words: Vec<String> = args.collect();
            let dir = match state_dir() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("alerts: {e}");
                    std::process::exit(1);
                }
            };
            match alerts::cli(&words, &dir, &SystemOut, &RealClock) {
                Ok(text) => print!("{text}"),
                Err(text) => {
                    eprint!("{text}");
                    std::process::exit(2);
                }
            }
        }
        other => {
            eprintln!("alerts: unknown command {other:?} (expected: status, cli)");
            std::process::exit(2);
        }
    }
}
