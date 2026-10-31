//! `hello greet`: read the run context, push a greeting widget.

use garage_plugin_sdk::{read_context, text_widget, Api, Value};

fn main() {
    if let Err(e) = run() {
        eprintln!("hello: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = read_context()?;
    let who = ctx.pointer("/focus/session/id").or_else(|| ctx.pointer("/focus/workspace")).and_then(Value::as_str).unwrap_or("garage");
    Api::from_env()?.put_widget("greeting", &text_widget(&format!("hello, {who}")))?;
    Ok(())
}
