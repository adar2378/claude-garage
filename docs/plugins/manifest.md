# Manifest reference

Every plugin has a file named `garage-plugin.toml` in its folder. Keys are `snake_case`. An unknown key is an error, so a typo shows at install and not at run time.

garage checks the whole file before anything runs. Each error names the field, for example `actions[1].command: missing`.

## A full example

```toml
#:schema https://raw.githubusercontent.com/adar2378/claude-garage/main/schemas/garage-plugin.schema.json
id = "usage-bar"
name = "Usage bar"
version = "0.1.0"
api_version = 1
platforms = ["macos", "linux"]
description = "5h and weekly usage"
requires = ["widgets.click_args"]
permissions = ["ui.widgets", "ui.panes"]

[[actions]]
id = "details"
title = "Usage details"
contexts = ["global"]
pane = "details"
default_key = "u"

[[events]]
on = "timer"
every_secs = 60
command = ["{plugin_dir}/bin/usage", "refresh"]

[[panes]]
id = "details"
title = "Usage details"
placement = "overlay"
command = ["{plugin_dir}/bin/usage", "ui"]
size = { cols = 80, rows = 24 }

[[widgets]]
id = "bar"
slot = "strip.right"

[[cli]]
name = "usage"
description = "Print usage"
command = ["{plugin_dir}/bin/usage", "cli"]
```

## Rules

- **`id`** matches `^[a-z][a-z0-9-]{1,30}$`, is unique, and is not a reserved word (`session`, `workspace`, `focus`, `jump`, `overlay`, `view`, `hooks`, `app`, `pet`, `daemon`, `chord`, `plugin`, `pane`, `core`).
- **Local ids** (action, pane, widget) match `^[a-z][a-z0-9_-]{0,31}$`. No dots or colons. The full id of an action is `<plugin>:<id>`.
- **`api_version`** must be one garage supports. See [versioning](versioning.md).
- **`requires`** lists feature names from `GET /api/plugins/features`. If one is missing garage refuses to load the plugin and says which. Use `name` for a flag, or `name:value` for a list, for example `events:session.turn` or `slots:tile.gutter`.
- **Permissions** must be honest: a manifest with `[[widgets]]` must declare `ui.widgets`, with `[[panes]]` must declare `ui.panes`, and with a button that runs a core action must declare `ui.navigate` (navigation only) or `core.actions`.
- **Actions**: exactly one of `command` and `pane`.
- **Events**: `timer` needs `every_secs` of at least 5. Other events must not have it.
- **Buttons and clicks**: `on_click` is a core id, one of this plugin's own action ids (`this-plugin:action` or the bare local id), never another plugin's.
- **`[[cli]]`**: the name must not be a core command (`tui`, `daemon`, `restart`, `plugin`, `host`, `help`, `version`), a name kept free for core (`start`, `stop`, `status`), or another plugin's. Only one installed plugin may set `fallback = true`.
- **`[release]`** needs `checksums` and at least one asset. Keys are `<macos|linux|windows>-<aarch64|x86_64>`.
- **Slots**: `rail.row`, `tile.title.right` and `tile.gutter` are about one session. `rail.header` and `rail.extra_rows` are about one workspace. `strip.right` is global.

## Commands and variables

A command is an array: the program, then its arguments. There is no shell, so `$(x)` or `*` stay as typed. If you want a shell, write `["sh", "-c", "..."]`; the install preview prints such commands in full.

These words are replaced inside each element:

| Word | Value |
|---|---|
| `{plugin_dir}` | the plugin's folder |
| `{bin}` | the downloaded binary (see `[release]`) |
| `{state_dir}` | the plugin's [storage](storage.md) folder |
| `{platform}` | for example `macos-aarch64` |

A first element with no slash is looked up on `PATH`. One that starts with `./` or `{plugin_dir}` is looked up in the plugin folder.

In `[[build]]` commands `{plugin_dir}` is a staging folder that is moved afterwards. Do not record that path in build output; use paths relative to the plugin folder.

## Key reference

The tables below are generated from the same JSON schema that editors use. `claude-garage plugin schema` prints it, and it lives in [`schemas/garage-plugin.schema.json`](../../schemas/garage-plugin.schema.json).

<!-- generated:manifest begin (UPDATE_DOCS=1 cargo test -p garage-plugin) -->

### Header

These keys sit at the top of the file.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `id` | string | yes |  | Unique id: `^[a-z][a-z0-9-]{1,30}$`, not a reserved word. |
| `name` | string | yes |  | Display name. |
| `version` | string | yes |  | Semver. |
| `api_version` | integer | yes |  | The plugin API major version. |
| `platforms` | list of one of `macos`, `linux`, `windows` | yes |  | Systems the plugin runs on. |
| `description` | string | no |  | One line for the install preview and `plugin list`. |
| `homepage` | string | no |  | A URL shown in the install preview. |
| `requires` | list of string | no | `[]` | Feature names from `/api/plugins/features` the plugin needs. |
| `permissions` | list of one of `sessions.read`, `sessions.input`, `ui.widgets`, `ui.panes`, `ui.invoke`, `core.actions`, `ui.navigate`, `ui.notice` | no | `[]` | What the plugin uses. Shown at install; the daemon checks it on API calls. |

### `[[actions]]`

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `id` | string | yes |  | Local id; the full id is `<plugin>:<id>`. |
| `title` | string | yes |  | Shown in help and the palette. |
| `contexts` | list of one of `global`, `workspace`, `session` | no | `["global"]` | What must be focused for the action to be available. |
| `command` | list of string | no |  | The program and arguments to run (no shell). Exactly one of `command` and `pane`. |
| `pane` | string | no |  | Open one of this plugin's panes instead of running a command. |
| `default_key` | string | no |  | A suggestion; never overrides an existing binding. |
| `confirm` | boolean | no | `false` | Ask the user before a plugin-origin trigger (click, `ui/action`) runs it. |
| `timeout_secs` | integer | no |  | 1..=300, default 30. |

### `[[events]]`

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `on` | one of `session.created`, `session.closed`, `session.turn`, `session.forgotten`, `session.status_changed`, `worktree.created`, `workspace.added`, `workspace.removed`, `daemon.started`, `timer` | yes |  | The event, or `timer`. |
| `every_secs` | integer | no |  | Only with `on = "timer"`; at least 5. |
| `command` | list of string | yes |  | The program and arguments to run (no shell). |
| `timeout_secs` | integer | no |  | 1..=60, default 10. |

### `[[panes]]`

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `id` | string | yes |  | Local id, used in `pane = "..."` and the open-pane route. |
| `title` | string | yes |  | Shown in the pane frame. |
| `placement` | one of `tile`, `overlay`, `popup` | yes |  | Where it appears. |
| `command` | list of string | yes |  | The program and arguments to run (no shell). |
| `singleton` | boolean | no | `true` | Opening again focuses the existing pane. Default true. |
| `size` | PaneSize | no |  | A size hint for overlay and popup panes. |

### `[[widgets]]`

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `id` | string | yes |  | Local id, used in the push route. |
| `slot` | one of `tile.title.right`, `tile.gutter`, `rail.row`, `rail.extra_rows`, `rail.header`, `strip.right` | yes |  | Where it is drawn; decides the scope key of its content. |

### `[[buttons]]`

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `slot` | one of `tile.title.right`, `tile.gutter`, `rail.row`, `rail.extra_rows`, `rail.header`, `strip.right` | yes |  | Where the button is drawn. |
| `label` | string | yes |  | The text of the button. |
| `tooltip` | string | no |  | Shown on hover. |
| `on_click` | string | yes |  | A core id, `plugin:action`, or a bare local action id. |
| `args` | any JSON | no |  | JSON passed as `trigger.args`. |
| `confirm` | boolean | no | `false` | May only add confirmation; a core action's own cannot be removed. |
| `priority` | integer | no | `0` | Higher survives narrow widths. |

### `[[cli]]`

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `name` | string | yes |  | The subcommand: `claude-garage <name>`. |
| `description` | string | no |  | Shown in help. |
| `command` | list of string | yes |  | The program and arguments to run (no shell). |
| `fallback` | boolean | no | `false` | Also receives an unknown first word. At most one installed plugin. |
| `needs_daemon` | boolean | no | `false` | `claude-garage` makes sure a current daemon is running before the command starts (the command calls the daemon API). |

### `[[build]]`

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `platforms` | list of one of `macos`, `linux`, `windows` | no | `[]` | Default: all header platforms. |
| `command` | list of string | yes |  | The program and arguments to run (no shell). |
| `timeout_secs` | integer | no |  | Default 600. |

### `[release]`

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `repo` | string | no |  | Default: the repo the plugin was installed from. |
| `tag` | string | no |  | Template; `{version}` is replaced. |
| `checksums` | string | yes |  | Asset holding `<sha256> <asset>` lines. |
| `assets` | table of string | no | `{}` | `os-arch` (for example `macos-aarch64`) to asset name. |

<!-- generated:manifest end -->
