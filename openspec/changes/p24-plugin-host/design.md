## Context

p21 gave us a Rust daemon (`crates/daemon`: `AppState`, `EventBus`/`DaemonEvent`, axum routes under `api/`, `SessionBackend` with `AttachSpec`, `Registry` over `state.json`, `Config` with `garage_dir`). p22 gave the TUI (`wall/`) its seams: `actions` (`ActionId`, `ActionCtx`, `ActionSource`), `keymap` (`bind`, `bind_override`), `slots` (`SlotId`, `SlotItem`, `SlotProvider`, `Placement`, `layout`), `runtime/dispatch.rs` and `ui/hit_targets.rs`. `claude-garage plugin` exists only as a stub that bails (`crates/cli/src/main.rs`).

This design makes the plugin host real. It has three halves: a daemon half (registry, installer, runner, widget store, panes), a TUI half (plugin-fed slot provider, plugin actions in keymap and dispatch, click targets with payloads) and a contract half (manifest, JSON schema, versioned API, features endpoint).

The decisions below the "Decisions made earlier" heading were fixed before this design and are kept as is. Everything after them is new.

## Decisions made earlier

**1. Out-of-process plugins, any language.** A plugin is a folder with `garage-plugin.toml` and commands run as argv arrays (no shell in between unless the command starts one). No in-process code, no plugin SDK required. Alternatives: Lua scripting, WASM. Deferred: both cost weeks and create a larger compatibility surface.

**2. Manifest shape (`garage-plugin.toml`).**
- Header: `id`, `name`, `version`, `api_version`, `platforms`, `description`, `permissions`.
- `[[actions]]`: `id`, `title`, `contexts`, `command`. Bindable to keys and buttons.
- `[[events]]`: `on` (for example `session.closed`, `session.status_changed`, `worktree.created`), `command`.
- `[[panes]]`: `id`, `title`, `placement` (`tile` | `overlay` | `popup`), `command`. Runs as a backend session.
- `[[widgets]]`: `id`, `slot` (`rail.row`, `tile.title.right`, `strip.right`, ...). Content is pushed at runtime over the daemon API.
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

## Goals / Non-Goals

**Goals**
- A plugin author can add an action, a key, a button, a widget, an overlay pane, an event hook and a CLI subcommand with a TOML file and any executable.
- A plugin with no commands at all (the close-button example) is valid: buttons can bind core actions.
- The core stays unchanged for users with no plugins installed: no new frames, no new keys, no new threads that run.
- The five known p22 gaps are closed in the core seams, not worked around in plugin code.

**Non-Goals (v1)**
- A sandbox, a plugin marketplace, plugin signing, or a central registry.
- In-process code, scripting or WASM.
- Long-running plugin services managed by the daemon (see Open questions; panes and timers cover the need).
- Windows and Linux process semantics beyond keeping the code paths portable (p26 does the work; this design names the places that need an OS seam).
- Persisting widget content across daemon restarts.

## Design

### 1. Where plugins live on disk

All under `Config.garage_dir` (`~/.garage`, or `GARAGE_DIR`):

```
plugins/
  plugins.json            registry of installed/linked plugins (atomic write: tmp + rename)
  installed/<id>/         checkout (or copy) of a GitHub plugin, plus built/downloaded binaries
  data/<id>/              GARAGE_PLUGIN_STATE_DIR: plugin storage, survives update
  logs/<id>.log           stdout+stderr of every run (rotated: 1 MiB, 2 files)
  tokens/<id>             per-plugin API token, mode 0600
```

`plugins.json` is separate from the session registry (`state/registry.rs`, `state.json`) so a broken plugin file can never lose session metadata. Entry shape:

```json
{
  "version": 1,
  "plugins": [{
    "id": "close-button",
    "source": {"kind": "github", "repo": "owner/repo", "subdir": null, "ref": "v0.1.0", "commit": "9f2c..."},
    "enabled": true,
    "installedAt": 1761900000000,
    "manifestSha256": "ab12...",
    "grantedPermissions": ["core.actions"],
    "bindings": {"close-button:close": "x"}
  }, {
    "id": "dev-plugin",
    "source": {"kind": "link", "path": "/Users/me/src/dev-plugin"},
    "enabled": true
  }]
}
```

- A linked plugin reads its manifest from the source path; its data and logs still live under `data/<id>` and `logs/<id>.log`.
- `id` rule: `^[a-z][a-z0-9-]{1,30}$`, unique, and not equal to a reserved word (`session`, `workspace`, `focus`, `jump`, `overlay`, `view`, `hooks`, `app`, `pet`, `daemon`, `chord`, `plugin`, `pane`, `core`).
- `bindings` records user key choices (see section 8), so they survive `update`.

The daemon loads `plugins.json` at start (new `AppState.plugins: PluginHost`) and the CLI reads it directly for `list` and `[[cli]]` dispatch, so those work with the daemon down.

### 2. Manifest schema

New workspace crate `crates/plugin` (`garage-plugin`) holds the manifest types, validation and the schema generator. Daemon and CLI depend on it; the TUI depends only on `garage-protocol` for the widget wire types (section 5).

Full field list (all keys are `snake_case`; unknown keys are an error so typos surface):

```toml
# Header (required unless marked optional)
id          = "usage-bar"            # see id rule above
name        = "Usage bar"
version     = "0.1.0"                # semver
api_version = 1                      # integer: plugin API major version
platforms   = ["macos", "linux"]     # subset of macos | linux | windows
description = "5h and weekly usage"  # optional
homepage    = "https://..."          # optional
requires    = ["widgets.click_args"] # optional: feature names from /api/plugin/features
permissions = ["sessions.read", "ui.widgets"]

[[actions]]
id          = "refresh"              # local id; full id is "usage-bar:refresh"
title       = "Refresh usage"        # shown in help and the palette
contexts    = ["global"]             # global | workspace | session (what must be focused)
command     = ["{plugin_dir}/bin/usage", "refresh"]   # exactly one of command | pane
# pane      = "details"              # open one of this plugin's panes instead of running a command
default_key = "u"                    # optional suggestion; never overrides an existing binding
confirm     = false                  # ask before running (see section 9)
timeout_secs = 30                    # optional, 1..=300, default 30

[[events]]
on      = "session.status_changed"   # event name list in section 4; or "timer"
every_secs = 60                      # only with on = "timer", minimum 5
command = ["{plugin_dir}/bin/usage", "event"]
timeout_secs = 10                    # optional, 1..=60, default 10

[[panes]]
id        = "details"
title     = "Usage details"
placement = "overlay"                # tile | overlay | popup
command   = ["{plugin_dir}/bin/usage", "ui"]
singleton = true                     # default true: opening again focuses the existing one
size      = {cols = 80, rows = 24}   # optional hint for overlay/popup (percent of screen if "pct")

[[widgets]]
id    = "bar"
slot  = "strip.right"                # tile.title.right | rail.row | rail.header | strip.right | overlay(reserved)

[[buttons]]
slot     = "tile.title.right"
label    = " x "
tooltip  = "Close this session"      # optional
on_click = "session.close"           # core id, "plugin:action", or a bare local action id
args     = {}                        # optional JSON passed as trigger.args
confirm  = true                      # may only add confirmation; cannot remove a core action's own
priority = 5                         # optional, default 0; higher survives narrow widths

[[cli]]
name        = "usage"                # claude-garage usage ...
description = "Print usage"
command     = ["{plugin_dir}/bin/usage", "cli"]

[[build]]
platforms = ["macos", "linux"]       # optional filter, default: all in header platforms
command   = ["cargo", "build", "--release"]
timeout_secs = 600                   # default 600

[release]                            # optional; if the current target is listed, [[build]] is skipped
repo   = "owner/repo"                # optional, defaults to the install repo
tag    = "v{version}"                # optional template
checksums = "SHA256SUMS"             # asset holding "<sha256>  <asset>" lines; REQUIRED when [release] is present
[release.assets]
"macos-aarch64" = "usage-darwin-arm64"
"macos-x86_64"  = "usage-darwin-amd64"
"linux-x86_64"  = "usage-linux-amd64"
# a downloaded asset is saved to <plugin dir>/bin/<plugin id> and made executable;
# commands reference it as {bin}
```

Argv substitution (done on array elements, never through a shell): `{plugin_dir}`, `{bin}`, `{state_dir}`, `{platform}` (`macos-aarch64`). Anything else with braces is left alone. A command whose first element is relative and has no slash is looked up on `PATH`; one starting with `./` or `{plugin_dir}` is looked up in the plugin directory. Plugins that want a shell write `["sh", "-c", "..."]` and own the risk (the install preview prints such commands in full).

Validation (all done before anything runs, errors name the field path, for example `actions[1].command: missing`):
- types and unknown keys via serde `deny_unknown_fields`;
- `id` rules, unique local ids per kind, `on` is a known event name, `slot` is a known slot and the widget/button is valid for it (`rail.row` and `tile.title.right` are session-scoped, `rail.header` is workspace-scoped, `strip.right` is global);
- `on_click` resolves to a known core id, a plugin action, or a local action;
- exactly one of `command` / `pane` per action; `timer` events need `every_secs >= 5`;
- `[release]` requires `checksums`; `[[cli]]` name must not equal a core subcommand (`start`, `stop`, `status`, `restart`, `install`, `plugin`, ...) or another plugin's `[[cli]]` name;
- `requires` entries are known features, otherwise the plugin is refused with "needs garage feature X".

**JSON schema.** `schemars` derives `schemas/garage-plugin.schema.json` from the Rust types. `claude-garage plugin schema` prints it. A unit test regenerates it and fails if the committed file differs. Docs tell authors to put `#:schema https://raw.githubusercontent.com/<garage repo>/main/schemas/garage-plugin.schema.json` on the first line for editor autocomplete (TOML language servers read that directive).

### 3. Install, link, update, list, remove

All subcommands live in `crates/cli` and call a library API in `garage-plugin` (so they work without the daemon). After a change the CLI asks the daemon to reload (`POST /api/plugins/reload`); if the daemon is down, nothing more is needed because it loads at start.

**`plugin install owner/repo[/subdir] [--ref <tag|branch|sha>] [--yes] [--allow-dangerous]`**
1. Resolve the source. Only GitHub `owner/repo` shorthand and full `https://github.com/owner/repo` URLs in v1.
2. `git clone --depth 1 [--branch ref]` into a temp dir under `plugins/` (uses the same `git` binary approach as `crates/daemon/src/git.rs`; 60 s timeout). Record the commit sha. Refuse symlinks that leave the checkout.
3. Parse and validate `garage-plugin.toml` at `<subdir>/`. Check `platforms` contains the current OS and `api_version` is supported. Nothing from the plugin has run yet.
4. Print the **preview** (this is the security gate, see section 11):
   - name, version, source repo and commit, license file if present;
   - "Plugins are not sandboxed. They run as you and can read, write and delete anything you can.";
   - permissions, with `sessions.input` and `core.actions` marked DANGEROUS and explained in one line;
   - exactly what will run: each `[[build]]` command (or the release asset URLs and checksum file that will be downloaded), then each action/event/cli/pane command, as argv;
   - UI it adds: keys it suggests, buttons, widgets, panes;
   - if updating: a diff of permissions, commands and keys against the installed version.
5. Ask `Install? [y/N]`. `--yes` skips the question but still prints the preview; if any dangerous permission is declared `--allow-dangerous` is also required (so scripts cannot grant it by accident).
6. Get the binaries: if `[release.assets]` has the current `os-arch`, download it from `https://github.com/<repo>/releases/download/<tag>/<asset>` and the checksums file, verify sha256, store under `installed/<id>/bin/`. Otherwise run `[[build]]` commands in order in `installed/<id>/` with the build timeout, logging to `logs/<id>.log`. A build failure removes the half-installed directory and leaves the registry untouched.
7. Move the checkout to `installed/<id>/`, create `data/<id>/`, create the token, write `plugins.json` atomically with `grantedPermissions` and `manifestSha256`.

**`plugin link <path>`** registers a local folder with no build and no download. The preview and permission prompt still run (once, and again if the manifest's commands or permissions change on reload). Linked plugins show `dev` in `list`. The daemon polls linked manifests' mtime every 2 s and reloads on change (no file-watcher crate; the poller pattern already exists in `poller.rs`). Reload errors keep the previous working version loaded and show a toast.

**`plugin update [id|--all]`** re-clones at the stored ref (or the default branch if none), builds a new preview diff, and prompts only if permissions, commands, build steps or download URLs changed (a pure version/code change is applied without a prompt, but still shows the new commit). It stages into a temp directory and swaps atomically, so a failed update leaves the old plugin working. `data/<id>` is untouched. Links are skipped.

**`plugin list [--json]`** prints id, version, source (`github owner/repo@sha` or `dev path`), enabled, permissions, and any load problem (refused api_version, missing feature, key conflicts).

**`plugin remove <id> [--purge]`** disables the plugin in the daemon (kills running processes, closes panes, clears widgets and bindings, section 4), removes `installed/<id>`, the token and the registry entry. `data/<id>` and logs are kept unless `--purge`.

Extra utilities: `plugin enable|disable <id>`, `plugin logs <id> [-f]`, `plugin schema`, `plugin bind <action> <key>`.

### 4. How the daemon runs plugin processes

New module `crates/daemon/src/plugins/` (`registry.rs`, `runner.rs`, `events.rs`, `widgets.rs`, `panes.rs`, `api.rs`), owned by `AppState.plugins`.

**Spawning.** `tokio::process::Command` with the argv array, cwd = the plugin directory, stdin = the context JSON (closed after writing), stdout and stderr piped into `logs/<id>.log` with a `[run-id action]` prefix per line. Each run starts in its own process group/session (`setsid` on unix via an OS seam in `os/`) so a timeout or uninstall can kill the whole tree. Never through a shell. The environment is the daemon's environment plus:

| Variable | Value |
|---|---|
| `GARAGE_API` | `http://127.0.0.1:<port>` |
| `GARAGE_API_VERSION` | `1` |
| `GARAGE_PLUGIN_ID` | plugin id |
| `GARAGE_PLUGIN_VERSION` | manifest version |
| `GARAGE_PLUGIN_DIR` | plugin directory |
| `GARAGE_PLUGIN_STATE_DIR` | `plugins/data/<id>` (created, exists before first run) |
| `GARAGE_PLUGIN_TOKEN` | per-plugin token for `/api/plugins/...` calls |
| `GARAGE_CONTEXT` | the context JSON, if at most 32 KiB |
| `GARAGE_CONTEXT_FILE` | path to a temp file with it, when larger (the env var is then unset) |
| `GARAGE_TRIGGER` | `key`, `button`, `widget`, `palette`, `event`, `cli`, `timer`, `pane` |

**Context JSON** (stdin and `GARAGE_CONTEXT`), the same shape for actions, events, panes and CLI:

```json
{
  "api_version": 1,
  "plugin": {"id": "close-button", "version": "0.1.0", "state_dir": "/Users/me/.garage/plugins/data/close-button"},
  "trigger": {"kind": "button", "key": null, "slot": "tile.title.right", "action": "close-button:close", "args": {}},
  "focus": {
    "workspace": "garage",
    "session": {"id": "garage/main", "dir": "/Users/me/dev/garage", "status": "working", "branch": "main", "worktree": null}
  },
  "selection": {"session": "garage/main", "index": null, "view": null},
  "worktree": null,
  "event": null
}
```

- Built by the TUI from `ActionCtx` (workspace, session, index, selection, view, worktree, args, source) and completed by the daemon from its registry and status store (dir, status, branch). The TUI owns focus; the daemon does not know it, which is why the TUI sends the context rather than the daemon guessing.
- `event` is set for event hooks: `{"name": "session.closed", "at": 1761900000000, "data": {...}}`. `trigger.args` carries the payload of a clicked widget or button.
- Missing pieces are `null`, never absent, so plugin code can index without checks.

**Triggering an action.** TUI: `Effect::RunPluginAction{plugin, action, ctx}` (in `runtime/effects.rs`) calls `POST /api/plugins/{id}/actions/{action}/run` with the context. The daemon answers `202 {"run_id": "..."}` immediately and runs the process in the background. On non-zero exit, timeout or spawn failure it publishes `DaemonEvent::PluginRunFailed{plugin, action, reason}`; the TUI shows a toast ("close-button: close failed, see `plugin logs close-button`"). Stdout is log output only in v1; a plugin changes the UI through the API (widgets, panes), not by printing.

**Event hooks.** `DaemonEvent` today has only `StatusTransition` and `SessionsChanged` (no payload). The bus gains payload variants, produced where the knowledge lives:
- `SessionCreated{id, dir, workspace}` and `SessionClosed{id, dir, workspace, reason}`: the poller already diffs the live session set to emit `SessionsChanged`; it also emits the typed events from that diff. `SessionsChanged` stays for the existing SSE consumers.
- `WorktreeCreated{...}` from `api/worktrees.rs`/`sessions::create`; `WorkspaceAdded/Removed{name}` from `api/workspaces.rs`.
- `PluginRunFailed`, `WidgetsChanged{plugin}`, `PluginsChanged`, `PaneEvent` (plugin-host internal; all become SSE frames).

Plugin-visible event names (v1): `session.created`, `session.closed`, `session.status_changed`, `worktree.created`, `workspace.added`, `workspace.removed`, `daemon.started`, `timer`. The event dispatcher is one more subscriber of `EventBus`. For each matching `[[events]]` entry it enqueues a run on that plugin's bounded queue (64; when full the oldest queued run is dropped and logged). At most 4 runs per plugin execute at once. `timer` entries run on a tokio interval, skipped (not queued) if the previous run of that entry is still going. Event runs default to a 10 s timeout, actions 30 s, build 600 s; a CLI run is interactive and has no timeout. On timeout: `SIGTERM` to the process group, `SIGKILL` after 2 s. A plugin that fails 5 event runs in a row is auto-disabled for events until reload, with a toast and a log line (stops a broken hook from spawning forever).

**`[[cli]]`.** `crates/cli` adds clap `#[command(external_subcommand)] External(Vec<OsString>)`. The first word is looked up in `plugins.json` (+ manifests); core subcommands always win. The command runs with inherited stdio, the same env (the context has `trigger.kind = "cli"`, the cwd as `focus` directory and no session), and the exit code is passed through. It needs the daemon only if the plugin calls the API.

**Uninstall / disable.** Order: mark disabled so new triggers are refused; kill process groups of all running runs; kill and unregister pane sessions; drop its widgets and publish `WidgetsChanged`; publish `PluginsChanged` so the TUI unbinds its keys and removes its buttons; then remove files.

### 5. Widgets: how content reaches the TUI

**Plugin to daemon.** A plugin pushes content for one of its declared widgets:

```
PUT /api/plugins/{id}/widgets/{widget}      Authorization: Bearer $GARAGE_PLUGIN_TOKEN
{
  "scopes": {                               // keys depend on the widget's slot, see below
    "garage/main": [ <item>, ... ],
    "*": [ <item> ]                         // "*" = the slot has no scope (strip.right)
  },
  "ttl_secs": 120                           // optional: content disappears after this
}
DELETE /api/plugins/{id}/widgets/{widget}   // clear all
```

Scope keys: session id for `tile.title.right` and `rail.row`; workspace name for `rail.header`; `"*"` for `strip.right`. A PUT replaces the widget's whole content (no patch protocol in v1; simpler to reason about and size-limited). A scope missing from the map shows nothing.

**Item and segment format (v1)** lives in `garage-protocol` so daemon and TUI share the type:

```json
{
  "priority": 10,
  "placement": {"kind": "flow"},
  "segments": [
    {"text": "5h ", "style": {"fg": "dim"}},
    {"text": "42%", "style": {"fg": "warn", "bold": true}, "click": {"action": "usage-bar:details", "args": {"window": "5h"}}}
  ]
}
```

- `placement`: `{"kind":"flow"}` (default) or `{"kind":"filler","x":2,"caption":"..."}` (strip only, see section 7).
- `style` (all optional): `fg` / `bg` are a theme token (`text`, `dim`, `accent`, `ok`, `warn`, `error`) or `#rrggbb`; booleans `bold`, `italic`, `underline`, `dim`. Theme tokens map to `ui/theme.rs` so plugins follow the user's theme; hex values are snapped to the terminal's colour depth by the renderer.
- `click` (optional, per segment): `{"action": "<id>", "args": <any JSON>}`. `action` is a core id, `plugin:action` or a bare local id (resolved against the owning plugin).
- Segments with the same `click` and adjacent columns form one click range.

**Limits (daemon rejects with 413/422 and a message naming the limit):** 16 items per scope, 16 segments per item, 64 display columns of text per item, 512 bytes of `click.args`, 16 KiB total request body, 256 scopes per widget. Text is sanitised: control characters and newlines are dropped, width is measured with the same unicode-width rules ratatui uses.

**Rate limits.** Per plugin: token bucket of 10 PUTs per second (burst 20); more gets `429` with `Retry-After`. A PUT whose body hashes equal to the stored content is a no-op (no event). Accepted changes are coalesced: the daemon emits at most one `WidgetsChanged{plugin}` per plugin per 200 ms (so at most 5 SSE frames per second per plugin), and the TUI redraws at most once per frame as it does now.

**Daemon to TUI.** The daemon keeps `WidgetStore` in `AppState.plugins` (in memory only). The SSE stream gains `event: widgets` with `{"plugin": "usage-bar", "rev": 42}` and `event: plugins` (`{}`, installed set or bindings changed). The TUI then `GET /api/plugins/widgets` for the full snapshot (`{rev, widgets: [{plugin, widget, slot, scopes}], buttons: [...], actions: [...], bindings: [...]}`); the TUI never trusts an SSE payload for content, same pattern as `sessions` today. On SSE reconnect the TUI refetches. A lagged subscriber gets a `plugins` frame (the existing lag rule in `api/events.rs`).

**TUI side.** One `PluginSlotProvider` per slot is registered into `SlotRegistry` (next to `SlotRegistry::core()`); all share an `Arc<RwLock<WidgetSnapshot>>` that the SSE/refetch task updates, so drawing never blocks on I/O. `items(&SlotCtx)` picks the scope from `ctx.session` / `ctx.group` and maps each stored item to a `SlotItem` (styles to ratatui `Style`, clicks to `ClickTarget`). Items from several plugins in one slot are ordered by (`priority` desc, plugin install order); that resolves "two plugins claim the same slot" (the open item from the earlier decisions): both show, ordering is deterministic, and the existing width-budget `layout` drops the lowest priority first. Buttons are static `SlotItem`s from the manifest, registered by the same provider. With no plugins installed the provider returns empty vectors and nothing changes on screen.

**Expiry.** A widget with `ttl_secs` is dropped by a daemon timer when it elapses (publishing `WidgetsChanged`). A widget without a ttl stays until replaced, cleared, or the plugin is removed/disabled or the daemon restarts. Plugins are expected to re-push on `daemon.started`.

### 6. Panes: plugin UI as backend sessions

A pane is one process started through `SessionBackend::create` (tmux today, the Windows host later), so the TUI attaches with `attach_spec` into the same PTY tile code as agent sessions: resize, mouse, scrollback and key forwarding come for free.

- **Id and visibility.** Backend id `plugin.<plugin-id>.<pane-id>[.<n>]`. The session registry marks these `kind: "plugin-pane"`; `GET /api/sessions`, the rail, restore and the hooks path ignore them. They are listed at `GET /api/plugins/panes` with attach info. The poller's session diff skips them so they never emit `SessionClosed`.
- **Opening.** From a plugin action with `pane = "id"`, from another plugin command via `POST /api/plugins/{id}/panes/{pane}/open` (token-authenticated; body is an optional context), or from a key/button. The daemon creates the backend session with the plugin's env (the context JSON is passed in `GARAGE_CONTEXT` like any run; for a pane it is also the only way to tell it which session to act on), then publishes `PaneOpened{plugin, pane, session_id}`. The TUI receives the SSE `plugins` frame/`pane` event, fetches the attach spec and shows it.
- **Placement.**
  - `tile`: added to the wall grid of the current view as an extra tile with a plugin-coloured title; counts toward grid layout (`grid_layout.rs`) but is excluded from salience and triage.
  - `overlay`: a new overlay kind (`OverlayKind::PluginPane`) drawn as a large modal PTY. While it is open the key layer is the pane: all keys go to the PTY except the reserved close chord (`ctrl+q` twice by default, shown in the overlay frame). Clicks outside close it like other overlays.
  - `popup`: a small centred modal PTY (default 60x16) that closes when its process exits, like a menu or picker.
- **Lifecycle.** `singleton = true` (default) reuses a live pane. A pane that exits closes overlay/popup; a `tile` pane stays as "exited" with a restart action until closed. Disabling or removing the plugin kills its panes. Closing the TUI leaves panes running (tmux owns them) until the daemon stops them or the process ends, mirroring agent sessions; stale `plugin.*` backend sessions with no registered plugin are killed at daemon start.
- **Sizing.** The pane process learns its size from the PTY (`SIGWINCH`), nothing special.
- **Selection of a target.** An overlay such as a session picker needs to say "jump to this session": it calls `POST /api/plugins/ui/action` (permission `ui.invoke`) with an action id and args, which the daemon forwards as an SSE `invoke` frame; the TUI runs it through the normal dispatcher with `ActionSource::Plugin`. This is the one path by which a plugin process asks the TUI to do something, and it is subject to the same allowlist and confirmation rules as a click (section 9).

### 7. TUI integration: actions, keys, slots, dispatch

This section is also where the p22 review gaps are closed. Each is a change to the existing seam, made first (tasks group 1) and covered by unit tests with no plugins present.

**Gap A: runtime action ids.** `ActionId(pub &'static str)` is `Copy` and used by value across `keymap`, `dispatch`, `hit_targets` and `slots`. Plugin ids exist only at runtime. Decision: keep `ActionId` as a `Copy` newtype over `&'static str` and add `ActionId::intern(&str) -> ActionId`, backed by a process-wide interner (`Mutex<HashSet<&'static str>>`, strings leaked once). The set is bounded by the installed plugins' declared action ids (reload of an unchanged manifest hits the interner and leaks nothing), so the leak is a few hundred bytes. Alternatives rejected: `ActionId(Cow<'static, str>)` (no longer `Copy`, touches every call site and every `match` in dispatch for no behavioural gain) and `Arc<str>` (same). The shipped `ids::ALL` list stays core-only. Plugin ids are written `plugin:action` (colon), which cannot collide with core dotted ids; core ids never contain a colon.

**Gap B: click payloads.** `SlotItem.on_click: Option<ActionId>` and `Placed.on_click` become `Option<ClickTarget>`:

```rust
pub struct ClickTarget { pub action: ActionId, pub args: Option<Arc<serde_json::Value>> }
```

`Laid::clicks` returns `(Range<u16>, ClickTarget)`. `ActionCtx` gains `args: Option<Arc<Value>>` and `ActionSource` gains `Widget { plugin: String, slot: SlotId }` (click on a plugin-provided item) and `Plugin { plugin: String }` (a plugin's `ui/action` request). Core providers keep passing `ClickTarget::from(ids::X)` with `args: None`. `with_args` is the new builder next to `with_session` / `with_selection`.

**Gap C: clicks on every slot.** Today only `strip.right` registers click regions. The renderers for `tile.title.right`, `rail.row` and `rail.header` call `Laid::clicks(offset, limit)` too and push `SlotClickRegion { rect, target, session: Option<String>, workspace: Option<String> }` into `ClickMap` (new field `slot_clicks`). `hit_targets::action_at` tests `slot_clicks` before the tile focus/rail row tests (only when no overlay is open, the same barrier rule as the strip), and builds the `ActionCtx` from the region's session/workspace, not from the focus, so clicking the close button on an unfocused tile closes that tile. A click on a plugin button does not move focus unless the action does. `rail_target_at` already counts header rows from the registry, so header buttons keep the row maths consistent.

**Gap D: more than one filler.** `Placement::Filler` is "at most one item shows". It becomes "any number show": the strip sorts fillers by `x`, draws each at its column, and where two overlap or the caption would collide it drops the lower-priority one (ties: later). A filler never overwrites a flow item. Core pit pet behaviour is unchanged because it is the only filler without plugins.

**Gap E: dangerous and chord-only actions, confirm step.** The restart actions (`session.restart`, `workspace.restart`, `daemon.restart`) run only for `ActionSource::Chord`. Plugins cannot produce a chord and should not be able to reach a restart in one click. Introduce an action metadata table next to `ids` (`ActionMeta { id, dangerous: bool, plugin_invocable: bool }`):
- `plugin_invocable` core ids (allowlist): `session.close`, `session.focus`, `session.engage`, `session.maximize`, `session.spawn`, `session.spawn.worktree`, `session.detach`, `session.open_window`, `session.restore`, `session.restore_all`, `workspace.focus`, `focus.cycle.*`, `jump.blocked`, `view.cycle`, `view.focus`, `overlay.open.*`, `pet.pet`, `hooks.install`.
- `dangerous` ones: `session.close`, `session.restart`, `workspace.restart`, `workspace.remove`, `workspace.remove.kill`, `daemon.restart`, `session.restore_all`, `hooks.install`. Restart and remove ids are plugin-invocable only with a confirm step.
- `app.quit`, `overlay.close`, `overlay.select.*`, `chord.restart` are never plugin-invocable.

Dispatch rule: when the source is `Widget`/`Plugin` (or a key bound by a plugin) and the target is `dangerous`, the dispatcher does not run it. It arms it through the existing armed-action state (`state/armed_action.rs`): the first click or key shows "Close garage/main? click again or press y" for 3 s, and the second matching trigger (or `y`) runs it with the real source the core action requires (the restart family accepts `ActionSource::Plugin{confirmed: true}` in addition to `Chord`). The manifest `confirm = true` on a button or action adds a confirm step to a non-dangerous target but never removes the mandatory one. This is what the "close button" example relies on. Plugin processes (not core actions) with `confirm = true` use the same armed state before the TUI sends `RunPluginAction`.

**Keys.** `PluginsChanged` makes the TUI rebuild its keymap: start from `Keymap::defaults()`, then for each plugin action with a key (user binding from `plugins.json` `bindings`, else `default_key`) call `bind`. The existing rule "the first binding for a key wins" means a plugin never displaces a core key by accident. A user binding (via `claude-garage plugin bind <action> <key>`) uses `bind_override` and can displace anything, including a core key (printed as a warning). A conflicting `default_key` between two plugins goes to the plugin installed first; the loser is listed under "conflicts" in `plugin list` and in the help overlay. Plugin keys bind in the normal wall layer only in v1 (not overlays, not the restart chord layer). `contexts` gates availability: a `session` action with no focused session is a no-op with a hint toast.

**Dispatch.** `dispatch_action(id, ctx)` first asks "core?" (the `ids` match it has today); an unrecognised id that parses as `plugin:action` and is in the plugin snapshot becomes `Effect::RunPluginAction` (or `Effect::OpenPluginPane` for `pane` actions). Anything else is a no-op logged at debug level. The help overlay lists plugin actions and keys in a "Plugins" section grouped by plugin.

### 8. Storage and plugin-side API

Each plugin gets `plugins/data/<id>` (also `GARAGE_PLUGIN_STATE_DIR`). Garage never reads or migrates its contents. `remove --purge` deletes it.

Plugin-facing HTTP surface (all under `127.0.0.1`, plugin routes require the bearer token; existing core routes are unchanged and, as today, need no token from loopback callers):

| Route | Purpose | Permission checked |
|---|---|---|
| `GET /api/plugin/features` | versions and optional features (no token) | none |
| `PUT/DELETE /api/plugins/{id}/widgets/{widget}` | push or clear widget content | `ui.widgets` |
| `POST /api/plugins/{id}/panes/{pane}/open`, `DELETE ...` | open or close a pane | `ui.panes` |
| `POST /api/plugins/ui/action` | ask the TUI to run an action | `ui.invoke` |
| `POST /api/sessions/{id}/input` (new) | type text (and optionally Enter) into a session; needs a new `SessionBackend::send_text` | `sessions.input` |
| existing `GET /api/sessions`, `GET /api/workspaces`, `GET /api/events` | read state, subscribe | `sessions.read` / `workspaces.read` (soft, see section 11) |
| existing workspace and session write routes | create, remove | `workspaces.write`, `sessions.manage` |

A plugin that wants live events without the hook mechanism can also open `GET /api/events` itself (the `event: status` / `event: sessions` frames, plus the new typed frames).

### 9. API versioning and feature checks

- `api_version` is an integer major. Garage supports a set (`[1]` now). A manifest asking for a version outside the set is refused at load, with "needs garage >= X" when it is newer (the daemon knows the minimum garage version per api version) or "uses removed API v0" when older.
- Within a major, changes are additive only: new events, new slot ids, new optional fields, new routes. Removing or changing meaning needs api 2 and a deprecation period in which garage supports [1, 2].
- `GET /api/plugin/features`:

```json
{
  "api_versions": [1],
  "garage": "0.7.0",
  "features": {
    "events": ["session.created", "session.closed", "session.status_changed", "worktree.created", "workspace.added", "workspace.removed", "daemon.started", "timer"],
    "slots": ["tile.title.right", "rail.row", "rail.header", "strip.right"],
    "widgets.click_args": true,
    "widgets.filler": true,
    "panes.placements": ["tile", "overlay", "popup"],
    "sessions.input": true,
    "actions.core_invocable": ["session.close", "..."],
    "limits": {"widget_items": 16, "widget_segments": 16, "widget_cols": 64, "widget_body_bytes": 16384, "widget_puts_per_sec": 10}
  }
}
```

- Manifest `requires = ["feature", ...]` is checked at install and load against the keys of `features` (booleans must be true; list-valued features are checked by `name:value`, e.g. `events:session.closed`). A plugin can also call the endpoint at runtime to adapt.
- v1 stays "preview" until p25's first-party plugins all work; the freeze rule in the earlier decision stands. Until then the docs say API v1 may still change between 0.x releases.

### 10. Worked example: the close-button plugin

No code, no commands, no permissions beyond triggering a core action. It adds a button to each tile's title bar that closes that session, with confirmation (a "close" is destructive, so the core makes it a confirmed action anyway).

```toml
#:schema https://raw.githubusercontent.com/OWNER/claude-garage/main/schemas/garage-plugin.schema.json
id          = "close-button"
name        = "Close button"
version     = "0.1.0"
api_version = 1
platforms   = ["macos", "linux", "windows"]
description = "Adds a close button to every tile title bar."
permissions = ["core.actions"]

[[buttons]]
slot     = "tile.title.right"
label    = " x "
tooltip  = "Close this session"
on_click = "session.close"
confirm  = true
priority = 5
```

What happens: install preview says "runs no commands, downloads nothing; adds a button in tile.title.right that triggers the core action session.close (DANGEROUS: closes a session; will ask for a second click)". On load the TUI's plugin provider adds a `SlotItem` " x " to every tile. A click on it is hit-tested through `slot_clicks`, so the `ActionCtx.session` is the clicked tile's session even when another tile is focused. `session.close` is `dangerous`, so dispatch arms it ("Close garage/main? click again"), and the second click runs the core close.

### 11. Security

Plugins are not sandboxed and cannot be: they are arbitrary executables running as the user. Anything they can do, you can do. What we do instead:

1. **Honest consent.** The install and update preview lists every command garage will ever run for the plugin (build, download URLs, action, event, pane and cli argv) before any of it runs, shows the commit, and states "not sandboxed" in plain words. Dangerous permissions are labelled and need `--allow-dangerous` in non-interactive installs.
2. **Pinned and re-consented.** Installs record the commit sha and manifest hash. Updates are explicit (`plugin update`), never automatic, and re-prompt if commands, permissions, build steps or download URLs changed. A manifest that changes under a linked path prompts again.
3. **No shell, argv only,** so a crafted session name or path in the context cannot become code through our own spawning. Context is passed as JSON on stdin/env, not interpolated into argv.
4. **Release downloads are verified.** Only from `github.com` releases of the declared repo, with a required checksums asset; a mismatch aborts the install. (The checksum file comes from the same release, so this guards corruption and CDN tampering, not a malicious author.)
5. **Permissions are declared and soft-enforced at our API.** Token-authenticated plugin routes refuse a call outside the declared permissions (403 naming the permission). This catches mistakes and keeps manifests honest, and it makes `plugin list` meaningful. It is not a security boundary: a hostile plugin can ignore its token and call the unauthenticated core routes on `127.0.0.1`, run `tmux`, or read `~`. The docs and the preview say so.
6. **Limits protect the UI, not the user:** widget size and rate caps, process timeouts, process-group kill, run concurrency and queue caps, auto-disable of a hook that fails repeatedly. A plugin cannot make the TUI hang by sending huge content, and a runaway child dies on uninstall.
7. **Dangerous core actions need a human.** Plugins cannot invoke restart/remove actions without the confirm step, and cannot invoke quit or overlay-internal actions at all (section 7, Gap E).
8. **Sensitive core features are not plugin territory** (p25: draft and screenshot recovery stay in the core).

Known residual risks, stated rather than hidden: the daemon API itself is unauthenticated on loopback for local processes (pre-existing, outside p24); a malicious plugin that passes review once can change behaviour in a later `update` that the user accepts without reading the diff; there is no central vetting.

## Risks / Trade-offs

- **Interned action ids leak a few bytes per distinct id.** Bounded by installed manifests; accepted over making `ActionId` non-`Copy`.
- **Spawn-per-event cost.** Each event starts a process. Fine for status changes (seconds apart); a plugin that needs a hot loop should use a pane or a timer. The 4-run cap and 64-entry queue bound the damage.
- **Widgets are replace-only and in-memory.** Simple and bounded, at the cost of re-pushing after a daemon restart (`daemon.started` event exists for that).
- **Soft permissions can give false comfort.** Mitigated by wording; the preview always leads with "not sandboxed".
- **Attach-based panes depend on the backend.** On the Windows host (p26) `AttachSpec::Socket` carries them; no p24 code should assume tmux outside `backend/tmux.rs`.
- **TUI owns focus, daemon runs the process.** The context is built in the TUI and trusted by the daemon only for non-authoritative fields (the daemon recomputes dir/status from its registry).

## Migration Plan

Additive. With `plugins/plugins.json` absent nothing is created and nothing changes. The `Plugin` stub subcommand is replaced by the real subcommands. The SSE stream gains frames (`widgets`, `plugins`, `pane`, `invoke`, `plugin_run_failed`) that older TUIs ignore. Rolling back = delete the `plugins/` directory.

## Open questions

1. **Long-running services.** Pushing a usage bar by `timer` events works for polling, but a plugin that wants a persistent process (websocket, file tail) has only panes. Add `[[services]]` (daemon-supervised, restart with backoff) in v1 or defer? Leaning defer until a p25 plugin needs it.
2. **Plugin keys vs the future user keymap.** `plugins.json` `bindings` is a stopgap. If p22/p27 add a user keymap file, plugin bindings should move there. Decide when that file is specified.
3. **Release asset trust.** A checksums file from the same release adds little against a malicious author. Do we want optional author signing (minisign) later? Not v1.
4. **Windows/Linux `{platform}` naming and `.exe` resolution** need to be fixed with p26; this design only keeps the substitution points.
5. **Daemon API authentication** (all callers, not just plugins) would make permissions enforceable for real; out of scope but it decides whether "soft" can ever become "hard".
6. **Widget scopes beyond session/workspace/global** (per-view, per-tile-size) are left out of v1; add on demand.
