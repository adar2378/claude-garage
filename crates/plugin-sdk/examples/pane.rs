//! A pane: pick one of the live sessions and focus it. Declare it under
//! `[[panes]]` (placement `overlay`) and give the plugin `ui.invoke` and
//! `ui.navigate`. Needs the `tui` feature:
//! `cargo run --example pane --features tui`.

use garage_plugin_sdk::tui::{Picker, run_picker};
use garage_plugin_sdk::{Api, Target, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api = Api::from_env()?;
    let sessions = api.sessions()?;
    let ids: Vec<String> = sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    if let Some(i) = run_picker(Picker::new("Jump to session", ids.clone()))? {
        api.ui_action("session.focus", Target::Session(&ids[i]))?;
    }
    Ok(())
}
