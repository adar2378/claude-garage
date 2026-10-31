//! An action that jumps to the focused session's workspace and says so.
//! Permissions: `ui.invoke`, `ui.navigate`, `ui.notice`.

use garage_plugin_sdk::{Api, Context, Target};

fn main() -> Result<(), garage_plugin_sdk::Error> {
    let ctx = Context::read()?;
    let api = Api::from_env()?;
    match ctx.session() {
        Some(session) => {
            api.ui_action("session.focus", Target::Session(session))?;
            api.notice(&format!("focused {session}"), None)?;
        }
        None => api.notice("no session is focused", None)?,
    }
    Ok(())
}
