//! `usage-bar refresh`: see the library for what it does.

use garage_plugin_sdk::Api;

fn main() {
    let result = match std::env::args().nth(1).as_deref() {
        Some("refresh") => Api::from_env().and_then(|api| {
            usage_bar::refresh(
                &api,
                &garage_plugin_sdk::state_dir()?,
                usage_bar::now_secs(),
            )
        }),
        other => {
            eprintln!("usage-bar: unknown command {other:?} (expected: refresh)");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("usage-bar: {e}");
        std::process::exit(1);
    }
}
