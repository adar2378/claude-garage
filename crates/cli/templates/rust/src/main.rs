//! @@TITLE@@: `greet` pushes one text item to the `status` widget.

use garage_plugin_sdk::{Api, Context, Error, text_widget};

/// The text the widget shows; `who` is the focused workspace, if any.
fn greeting(who: Option<&str>) -> String {
    format!("hello, {}", who.unwrap_or("garage"))
}

fn push(api: &Api, who: Option<&str>) -> Result<(), Error> {
    api.put_widget("status", &text_widget(&greeting(who)))
        .map(|_| ())
}

fn run() -> Result<(), Error> {
    let ctx = Context::read()?;
    push(&Api::from_env()?, ctx.workspace())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("@@ID@@: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::testing::FakeDaemon;

    #[test]
    fn greeting_names_the_workspace_or_falls_back() {
        assert_eq!(greeting(Some("app")), "hello, app");
        assert_eq!(greeting(None), "hello, garage");
    }

    #[test]
    fn push_puts_one_text_item_on_the_status_widget() {
        let daemon = FakeDaemon::start(|_| (200, r#"{"ok":true,"rev":1}"#.into()));
        push(&daemon.api("@@ID@@"), Some("app")).unwrap();
        let req = &daemon.requests()[0];
        assert_eq!(req.method, "PUT");
        assert_eq!(req.path, "/api/plugins/@@ID@@/widgets/status");
        assert_eq!(
            req.body["scopes"]["*"][0]["segments"][0]["text"],
            "hello, app"
        );
    }
}
