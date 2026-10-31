//! `prune-empty-projects event`: the `session.closed` hook.

use garage_plugin_sdk::{Api, Context};

fn main() {
    let result = (|| -> Result<(), String> {
        let ctx = Context::read().map_err(|e| e.to_string())?;
        let api = Api::from_env().map_err(|e| e.to_string())?;
        garage_plugin_prune_empty_projects::handle(
            &api,
            ctx.event_data(),
            garage_plugin_prune_empty_projects::SETTLE,
        )
        .map(|outcome| println!("{outcome:?}"))
        .map_err(|e| e.to_string())
    })();
    if let Err(e) = result {
        eprintln!("prune-empty-projects: {e}");
        std::process::exit(1);
    }
}
