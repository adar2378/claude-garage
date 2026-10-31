//! Push a plain-text widget. Run it through garage (it needs the plugin
//! environment): declare `[[widgets]] id = "greeting"` and the `ui.widgets`
//! permission in `garage-plugin.toml`.

use garage_plugin_sdk::{Api, Context, text_widget};

fn main() -> Result<(), garage_plugin_sdk::Error> {
    let ctx = Context::read()?;
    let who = ctx.workspace().unwrap_or("garage");
    let rev = Api::from_env()?.put_widget("greeting", &text_widget(&format!("hello, {who}")))?;
    eprintln!("pushed revision {rev}");
    Ok(())
}
