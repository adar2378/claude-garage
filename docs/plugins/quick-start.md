# Quick start: a plugin in 10 minutes

You will make a plugin that shows the number of live sessions in the status strip, and a key that refreshes it. You need garage running and `curl`, `jq` and a POSIX shell. The same plugin in Rust or TypeScript is shown at the end.

## 1. Make the folder (1 minute)

```sh
mkdir -p ~/plugins/count && cd ~/plugins/count
```

## 2. Write the manifest (2 minutes)

Create `garage-plugin.toml`:

```toml
#:schema https://raw.githubusercontent.com/adar2378/claude-garage/main/schemas/garage-plugin.schema.json
id = "count"
name = "Session count"
version = "0.1.0"
api_version = 1
platforms = ["macos", "linux"]
description = "Shows how many sessions are live"
permissions = ["ui.widgets"]

[[widgets]]
id = "total"
slot = "strip.right"

[[actions]]
id = "refresh"
title = "Refresh session count"
default_key = "N"
command = ["{plugin_dir}/count.sh"]

# Run on every status change, and once a minute as a backup.
[[events]]
on = "session.status_changed"
command = ["{plugin_dir}/count.sh"]

[[events]]
on = "timer"
every_secs = 60
command = ["{plugin_dir}/count.sh"]

# The daemon forgets widgets when it restarts. Draw again at once.
[[events]]
on = "daemon.started"
command = ["{plugin_dir}/count.sh"]
```

The first line is optional. It lets your editor check and complete the file.

## 3. Write the program (3 minutes)

Create `count.sh`:

```sh
#!/bin/sh
# garage gives us GARAGE_API and GARAGE_PLUGIN_TOKEN.
n=$(curl -s "$GARAGE_API/api/sessions" | jq 'length')
curl -s -X PUT "$GARAGE_API/api/plugins/count/widgets/total" \
  -H "authorization: Bearer $GARAGE_PLUGIN_TOKEN" \
  -H "content-type: application/json" \
  -d "{\"scopes\":{\"*\":[{\"segments\":[{\"text\":\"$n sessions\",\"style\":{\"fg\":\"dim\"}}]}]},\"ttl_secs\":300}"
```

Then `chmod +x count.sh`.

Two points:

- `scopes` is keyed by what the slot is about. `strip.right` is global, so the key is `"*"`. See [widgets](widgets.md).
- `ttl_secs` makes the widget vanish if the plugin stops pushing. Always set it for live data.

## 4. Link it (1 minute)

```sh
claude-garage plugin link ~/plugins/count
```

garage prints what the plugin asks for and what it will run. Answer `y`. The widget appears in the strip. Press `N` to refresh.

## 5. Look at the log if something is wrong (1 minute)

```sh
claude-garage plugin logs count -f
```

Everything the program prints (stdout and stderr) goes there. A run that exits with a non-zero code is marked failed. Five failed event runs in a row switch the plugin's event hooks off until you reload it.

## 6. Publish (1 minute)

Push the folder to a public GitHub repo. Others install it with `claude-garage plugin install you/count`. Add the topic `claude-garage-plugin` so it can be found. Before they install, they see your commands and permissions.

If your plugin needs a build step or a binary, see `[[build]]` and `[release]` in the [manifest reference](manifest.md).

## The same in Rust

```rust
use garage_plugin_sdk::{Api, Error, text_widget};

fn main() -> Result<(), Error> {
    let api = Api::from_env()?;
    let n = api.sessions()?.as_array().map_or(0, |a| a.len());
    let mut put = text_widget(&format!("{n} sessions"));
    put.ttl_secs = Some(300);
    api.put_widget("total", &put)?;
    Ok(())
}
```

## The same in TypeScript

```ts
import { Api, textWidget } from "@claude-garage/plugin-sdk";

const api = Api.fromEnv();
const n = (await api.sessions()).length;
await api.putWidget("total", { ...textWidget(`${n} sessions`), ttl_secs: 300 });
```

Set `command = ["node", "{plugin_dir}/count.mjs"]` in the manifest.

## Next

- Add an animation or a click: [widgets](widgets.md).
- Run something when a chat ends: [events](events.md).
- Open a full-screen picker: [panes](panes.md).
