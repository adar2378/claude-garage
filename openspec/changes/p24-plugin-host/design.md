> **Detailed design is written when the phase starts (after p21/p22).** This file records only the decisions already made.

## Decisions made

**1. Out-of-process plugins, any language.** A plugin is a folder with `garage-plugin.toml` and commands run as argv arrays (no shell in between unless the command starts one). No in-process code, no plugin SDK required. Alternatives: Lua scripting, WASM. Deferred: both cost weeks and create a larger compatibility surface.

**2. Manifest shape (`garage-plugin.toml`).**
- Header: `id`, `name`, `version`, `api_version`, `platforms`, `description`, `permissions`.
- `[[actions]]`: `id`, `title`, `contexts`, `command`. Bindable to keys and buttons.
- `[[events]]`: `on` (for example `session.closed`, `session.status_changed`, `worktree.created`), `command`.
- `[[panes]]`: `id`, `title`, `placement` (`tile` | `overlay` | `popup`), `command`. Runs as a backend session.
- `[[widgets]]`: `id`, `slot` (`rail.row`, `tile.title.right`, `strip.right`, …). Content is pushed at runtime over the daemon API.
- `[[buttons]]`: `slot`, `label`, `on_click` (an action id, core or plugin), `confirm`.
- `[[cli]]`: `name`, `command`. Exposed as `claude-garage <name>`.
- `[[build]]`: commands run once at install, per platform.
- `[release]`: per-OS binary asset names; garage downloads the right one at install, so authors need no install scripts.

**3. Plugin UI comes two ways.** Panes are backend sessions (same code path as agent tiles, so sizing, mouse and scrollback behave the same). Widgets are declarative segments plus click targets, kept to a tiny schema.

**4. Install from GitHub.** `plugin install owner/repo[/subdir] [--ref]` clones, shows the manifest summary and permissions, asks, builds or downloads, registers. `plugin link <path>` for development (no build, reload on change). `update`, `list`, `remove`.

**5. Permissions are declared, shown, not enforced.** Examples: `sessions.read`, `sessions.input` (flagged dangerous), `workspaces.write`, `network`. No sandbox: plugins run as the user. The install prompt says this in plain words.

**6. Context and storage.** Every action and event gets a context JSON (env var and stdin): focused session id, workspace, dir, worktree, selection, trigger (key, button, palette). Each plugin gets `GARAGE_PLUGIN_STATE_DIR`.

**7. Versioning.** `api_version` names the plugin API major version; plugins can query `/api/plugin/features` for optional capabilities. API v1 is frozen only after p25's plugins all work.

**8. Manifest schema.** Generated from the Rust types (schemars) and published, so editors autocomplete manifests.

## Open for the phase start
- Widget segment schema and update rate limits.
- How button clicks resolve when two plugins claim the same slot (order, priority).
- Whether `plugin link` reload watches files or needs a key.
