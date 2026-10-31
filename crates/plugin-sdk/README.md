# garage-plugin-sdk

Helpers for writing [claude-garage](https://github.com/adar2378/claude-garage) plugins in Rust.

A plugin is any program. garage runs it with an environment and a JSON run context, and the program
can call the daemon's HTTP API with its own token. This crate does the small, boring parts:

- read the run context (`read_context`, `Context`)
- call the daemon with the plugin token (`Api`)
- push widgets and notices, ask the wall to run actions, type into sessions, open panes
- keep state in the plugin's own folder (`read_state`, `write_state`, `StateLock`)
- hand a CLI run over to the wall (`request_wall`)
- optional: a fake daemon for tests (feature `testing`), a raw-mode picker and prompt for panes (feature `tui`)

The manifest, permissions, events and widget format are described in
[docs/plugins](https://github.com/adar2378/claude-garage/tree/main/docs/plugins).
Your `garage-plugin.toml` must declare the widget and the permission each call needs.

## A widget in a few lines

```toml
# garage-plugin.toml
id = "hello"
name = "Hello"
version = "0.1.0"
api_version = 1
platforms = ["macos", "linux"]
permissions = ["ui.widgets"]

[[actions]]
id = "greet"
title = "Say hello"
command = ["{plugin_dir}/bin/hello"]

[[widgets]]
id = "greeting"
slot = "strip.right"
```

```no_run
use garage_plugin_sdk::{Api, Context, text_widget};

fn main() -> Result<(), garage_plugin_sdk::Error> {
    let ctx = Context::read()?;
    let who = ctx.workspace().unwrap_or("garage");
    Api::from_env()?.put_widget("greeting", &text_widget(&format!("hello, {who}")))?;
    Ok(())
}
```

## Reacting to an event

```no_run
use garage_plugin_sdk::{Api, Context};

fn main() -> Result<(), garage_plugin_sdk::Error> {
    let ctx = Context::read()?;
    if ctx.event_name() == Some("session.closed") {
        let id = ctx.event_data()["id"].as_str().unwrap_or("a session");
        // needs the `ui.notice` permission
        Api::from_env()?.notice(&format!("{id} closed"), None)?;
    }
    Ok(())
}
```

## Testing without a daemon

Enable the `testing` feature in `[dev-dependencies]`:

```toml
[dev-dependencies]
garage-plugin-sdk = { version = "1", features = ["testing"] }
```

```ignore
use garage_plugin_sdk::{testing::FakeDaemon, text_widget};

let fake = FakeDaemon::start(|_req| (200, r#"{"ok":true,"rev":1}"#.to_string()));
let api = fake.api("hello");
api.put_widget("greeting", &text_widget("hi")).unwrap();
assert_eq!(fake.log(), ["PUT /api/plugins/hello/widgets/greeting"]);
```

## Versioning

The crate follows garage's version. It speaks plugin API v1, which only grows: see
[versioning](https://github.com/adar2378/claude-garage/blob/main/docs/plugins/versioning.md).
Core read routes (`sessions`, `workspaces`) return `serde_json::Value`, so a field that garage adds never breaks you.

## License

MIT
