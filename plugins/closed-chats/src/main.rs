//! `closed-chats sync` (event hook) and `closed-chats recent` (the H pane).

use garage_plugin_sdk::{Api, Context};

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    let result = match cmd.as_str() {
        "sync" => run_sync(),
        "recent" => run_recent(),
        other => Err(format!("unknown command {other:?} (sync | recent)")),
    };
    if let Err(e) = result {
        eprintln!("closed-chats: {e}");
        std::process::exit(1);
    }
}

fn run_sync() -> Result<(), String> {
    let ctx = Context::read().map_err(|e| e.to_string())?;
    let api = Api::from_env().map_err(|e| e.to_string())?;
    // The close event can beat the closed record to the daemon's registry.
    let expect = (ctx.event_name() == Some("session.closed"))
        .then(|| ctx.event_data()["id"].as_str().map(str::to_owned))
        .flatten();
    garage_plugin_closed_chats::sync(
        &api,
        expect.as_deref(),
        std::time::Duration::from_millis(150),
    )
    .map_err(|e| e.to_string())
}

fn run_recent() -> Result<(), String> {
    let api = Api::from_env().map_err(|e| e.to_string())?;
    garage_plugin_closed_chats::recent(&api).map_err(|e| e.to_string())
}
