//! `openspec-progress tick` (timer and startup hook) and
//! `openspec-progress overlay` (the pane).

use garage_plugin_sdk::{Api, Context, tui};

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    let result = match cmd.as_str() {
        "tick" => Api::from_env()
            .and_then(|api| garage_plugin_openspec_progress::tick(&api))
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "overlay" => run_overlay(),
        other => Err(format!("unknown command {other:?} (tick | overlay)")),
    };
    if let Err(e) = result {
        eprintln!("openspec-progress: {e}");
        std::process::exit(1);
    }
}

fn run_overlay() -> Result<(), String> {
    let ctx = Context::read().map_err(|e| e.to_string())?;
    let api = Api::from_env().map_err(|e| e.to_string())?;
    let lines = garage_plugin_openspec_progress::overlay(&api, ctx.workspace())
        .map_err(|e| e.to_string())?;
    tui::show(&lines, None).map_err(|e| e.to_string())
}
