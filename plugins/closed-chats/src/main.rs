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
    let state_dir = ctx.state_dir().ok_or("the run context has no state_dir")?;
    garage_plugin_closed_chats::sync_serialized(&api, std::path::Path::new(state_dir))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn run_recent() -> Result<(), String> {
    let api = Api::from_env().map_err(|e| e.to_string())?;
    garage_plugin_closed_chats::recent(&api).map_err(|e| e.to_string())
}
