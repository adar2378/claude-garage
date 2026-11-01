---
title: Build your first plugin
description: "Build a claude-garage plugin in about 20 minutes: scaffold it with plugin new, write the manifest and a widget, test it live and publish it to GitHub."
sidebar:
  order: 8
---

This walkthrough builds a plugin that shows how many sessions are live in the status strip. You will scaffold it, edit the manifest, write the widget code, try it live, test it and publish it. It takes about 20 minutes. The example is in Rust, with notes for TypeScript and shell.

You need claude-garage installed and running, and Rust (stable) for the Rust path. A plugin is a program that runs as you, so build only what you understand.

## What is a claude-garage plugin?

A plugin is a folder with a `garage-plugin.toml` file and any program. garage runs the program when something happens: a key press, a click, an event or a timer. The program gets a JSON run context and an API token, and it calls the daemon's HTTP API on `127.0.0.1` to put widgets in the wall. See [Plugins](/docs/guides/plugins/) for the ones that ship in the binary.

## Step 1: Scaffold the plugin

Pick a short id. It must be lowercase letters, digits and `-`, 2 to 31 characters, starting with a letter.

```bash
claude-garage plugin new session-count --lang rust
cd session-count
```

`--lang` is `rust` (the default), `ts` or `shell`. The command creates a working plugin with a manifest, code, a README, a `CLAUDE.md` of manifest rules and a test. It refuses to overwrite anything and checks the result with the same manifest checker that `plugin link` uses.

The scaffold adds one action ("Say hello") and one widget on the status strip. You will change both.

## Step 2: Read and edit the manifest

Open `garage-plugin.toml`. Replace the action and events so it looks like this:

```toml
#:schema https://raw.githubusercontent.com/adar2378/claude-garage/main/schemas/garage-plugin.schema.json
id          = "session-count"
name        = "Session count"
version     = "0.1.0"
api_version = 1
platforms   = ["macos", "linux"]
description = "Shows how many sessions are live in the status strip"
permissions = ["ui.widgets"]

[[build]]
command = ["cargo", "build", "--release"]

[[actions]]
id       = "refresh"
title    = "Refresh session count"
contexts = ["global"]
command  = ["{plugin_dir}/target/release/session-count"]

# Run when any session changes status.
[[events]]
on      = "session.status_changed"
command = ["{plugin_dir}/target/release/session-count"]

# Widgets live in the daemon's memory; push again whenever the daemon starts.
[[events]]
on      = "daemon.started"
command = ["{plugin_dir}/target/release/session-count"]

[[widgets]]
id   = "status"
slot = "strip.right"
```

Key points:

- **`id`** is unique and matches `^[a-z][a-z0-9-]{1,30}$`. Unknown keys are an error, so a typo shows at install and not at run time.
- **`api_version = 1`** is the frozen plugin API version. See [versioning](/docs/plugins/versioning/).
- **Commands are argument lists.** There is no shell. `{plugin_dir}` is replaced with the plugin folder.
- **`[[build]]`** runs at install and on every `plugin dev` reload.
- **`strip.right`** is a global slot, so its content uses the scope key `"*"`. Other slots are about one session or one workspace. See [Slots and widgets](/docs/plugins/widgets/).

The full list of keys is in the [manifest reference](/docs/plugins/manifest/).

## Step 3: Write the widget

Open `src/main.rs` and replace it with:

```rust
use garage_plugin_sdk::{Api, Error, Value, text_widget};

/// How many sessions the daemon listed.
fn count(sessions: &Value) -> usize {
    sessions.as_array().map_or(0, Vec::len)
}

fn push(api: &Api) -> Result<(), Error> {
    let n = count(&api.sessions()?);
    let mut put = text_widget(&format!("{n} sessions"));
    // The widget vanishes if the plugin stops pushing.
    put.ttl_secs = Some(300);
    api.put_widget("status", &put).map(|_| ())
}

fn main() {
    let result = Api::from_env().and_then(|api| push(&api));
    if let Err(e) = result {
        eprintln!("session-count: {e}");
        std::process::exit(1);
    }
}
```

The SDK does the plumbing:

- `Api::from_env()` reads `GARAGE_API`, `GARAGE_PLUGIN_ID` and `GARAGE_PLUGIN_TOKEN`, which garage sets for every run.
- `api.sessions()` calls `GET /api/sessions`.
- `text_widget` makes a widget body with one plain text item for every scope.
- `api.put_widget("status", ...)` calls `PUT /api/plugins/session-count/widgets/status`. The widget must be declared in the manifest, and the plugin needs `ui.widgets`.
- `ttl_secs` removes the widget if you stop pushing, so stale numbers never stay on screen. Set it for live data.

If your code needs to know why it ran, call `Context::read()`. It gives the focused workspace and session, the trigger and the event. The scaffold uses it in its `greet` example.

### The same in TypeScript or shell

The TypeScript SDK is `@claude-garage/plugin-sdk`. It needs Node 24 or newer and has no runtime dependencies:

```ts
import { Api, textWidget } from "@claude-garage/plugin-sdk";

const api = Api.fromEnv();
const n = (await api.sessions()).length;
await api.putWidget("status", { ...textWidget(`${n} sessions`), ttl_secs: 300 });
```

Set `command = ["node", "{plugin_dir}/dist/main.js"]` in the manifest. A `--lang ts` scaffold sets up the build for you. A `--lang shell` scaffold needs only `sh` and `curl`, and you push the widget with a `PUT` request and your bearer token.

## Step 4: Run it with plugin dev

```bash
claude-garage plugin dev
```

`plugin dev` links the folder, runs the `[[build]]` commands, watches for changes, rebuilds and reloads the plugin, and tails its log in your terminal. The first time, it shows what the plugin asks for and what it will run, and asks you to approve. Press `Ctrl+C` to stop it. The plugin stays linked. `claude-garage plugin link .` links without watching.

Open the wall. The count appears on the right of the status strip. Run "Refresh session count" from the palette, or bind a key:

```bash
claude-garage plugin bind session-count:refresh N
```

A manifest edit is checked again on reload, and every problem is printed with the field name, for example `actions[1].command: missing`. If something is wrong at run time, read the log:

```bash
claude-garage plugin logs session-count -f
```

A run that exits with a non-zero code is marked failed. Five failed event runs in a row switch the plugin's event hooks off until you reload it.

## Step 5: Choose permissions

`permissions` is what the plugin says it will use. garage shows it at install and checks it on API calls. A call without the right permission gets `403` and a message that names it.

| You want to | Permission |
|---|---|
| Put a widget in a slot | `ui.widgets` |
| Open your own pane | `ui.panes` |
| Show a toast | `ui.notice` |
| Jump to a blocked session | `ui.navigate` |
| Run any core action | `core.actions` (dangerous) |
| Type into a session | `sessions.input` (dangerous) |

Ask for the least. The widget in this example only needs `ui.widgets`. A manifest that uses a feature without its permission is refused. Two permissions are marked DANGEROUS in the install preview, and a script must pass `--allow-dangerous` to grant them.

Permissions are a declaration, not a sandbox. Your program can still read files and use the network. Read the [security model](/docs/plugins/security/). One rule to know early: a plugin can ask to close a session, but only the user can confirm it. Read [how the confirm rule keeps you in control](/blog/plugin-confirm-rule/).

## Step 6: Test it

The SDK has a fake daemon for tests (feature `testing`). It is a local HTTP server that answers from a closure and records every request, so a test needs no daemon and no tmux. Add this to `src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::{json, testing::FakeDaemon};

    #[test]
    fn counts_the_sessions_the_daemon_lists() {
        assert_eq!(count(&json!([{}, {}, {}])), 3);
        assert_eq!(count(&json!({})), 0);
    }

    #[test]
    fn pushes_one_text_item_to_the_status_widget() {
        let daemon = FakeDaemon::start(|req| {
            if req.method == "GET" {
                (200, r#"[{"id":"a"},{"id":"b"}]"#.into())
            } else {
                (200, r#"{"ok":true,"rev":1}"#.into())
            }
        });
        push(&daemon.api("session-count")).unwrap();
        let put = &daemon.requests()[1];
        assert_eq!(put.method, "PUT");
        assert_eq!(put.path, "/api/plugins/session-count/widgets/status");
        assert_eq!(put.body["scopes"]["*"][0]["segments"][0]["text"], "2 sessions");
    }
}
```

The SDK re-exports `json!` and `Value`, so you need no extra dependency. Run the tests:

```bash
cargo test
```

Test the logic that parses and counts, and test one request end to end against the fake. The scaffold's tests show the same pattern, and a TypeScript scaffold runs `npm test` and a shell scaffold runs `sh test.sh`.

## Step 7: Publish to GitHub

1. Push the folder to a public GitHub repository. Put `garage-plugin.toml` at the repository root. A plugin in a subfolder installs with `owner/repo/folder`.
2. Add the topic `claude-garage-plugin` to the repository. Then `claude-garage plugin search session` can find it.
3. Tag a version, for example `v0.1.0`, so people can pin it.
4. Others install it with:

   ```bash
   claude-garage plugin install you/session-count
   claude-garage plugin install you/session-count --ref v0.1.0
   ```

Before they install, garage shows them your commands and permissions. An update asks again only if commands, permissions or download addresses changed.

Two things to know for a Rust plugin:

- The SDK is not on crates.io. In your own repository the scaffold points at the tagged release on GitHub, so keep that dependency line.
- The `[[build]]` step runs `cargo build --release` on the user's machine. If you would rather ship a prebuilt binary, use `[release]` with a checksums file. garage checks the checksum. The [manifest reference](/docs/plugins/manifest/) lists the keys.

A short list for authors: ask for the least permission, never run `sh -c` with text you did not write, never print the token, and say honestly in `description` what the plugin does with the network.

## A larger worked example: OpenSpec progress

The repository has a real example in `examples/openspec-progress`. It shows, for every workspace that has `openspec/changes/*/tasks.md`, a `spec 12/30` widget in the rail header. Clicking it opens an overlay that lists each active change with a bar.

Its manifest uses more of the system than yours does:

- **A timer event.** `on = "timer"` with `every_secs = 5` re-reads the files. Five seconds is the minimum. There is no file watcher and no long-running process. Each run reads a few small files and exits.
- **A widget in `rail.header`.** That slot is about one workspace, so its scope keys are workspace names, not `"*"`.
- **A pane.** `placement = "overlay"` runs a program that draws a full-screen view. The action `openspec-progress:changes` opens it with `pane = "changes"`. The program uses the SDK's `tui` helpers.
- **Permissions** `ui.widgets` and `ui.panes`, and nothing more. It learns each workspace's folder from `GET /api/workspaces`, which needs no permission.

It also has tests that use the fake daemon, plus tests for edge cases such as symlinks that leave the workspace. Try it from a clone:

```bash
claude-garage plugin dev examples/openspec-progress
```

## Where to go next

- [Quick start](/docs/plugins/quick-start/): the same plugin in shell, in ten minutes.
- [Actions](/docs/plugins/actions/), [events](/docs/plugins/events/), [panes](/docs/plugins/panes/) and the [HTTP API](/docs/plugins/api/).
- [Contributing](/docs/guides/contributing/): work on garage itself.
