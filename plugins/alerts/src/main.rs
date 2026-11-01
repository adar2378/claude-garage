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
            if words.first().map(String::as_str) == Some("open") {
                // A clicked banner: never noisy. No wall or no daemon is fine.
                let result = Api::from_env()
                    .map_err(|e| e.to_string())
                    .and_then(|api| alerts::open(&words[1..], &api));
                if let Err(e) = result {
                    eprint!("alerts: {e}");
                    std::process::exit(2);
                }
                return;
            }
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
