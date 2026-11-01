## Why

People want to change how garage looks and behaves: add a button, show a usage bar, group sessions their own way, add a CLI command. Today every such change is a patch to the core, and the open contributor PR #2 shows the cost: 4,400 lines mixing platform work with product opinions in shared files. A plugin host lets opinions ship outside the core, so the core stays small and contributors stop editing the same files.

## What Changes

- A plugin is a folder with a `garage-plugin.toml` manifest and commands in any language. garage runs the commands as separate processes; plugins talk back through the daemon HTTP API and event stream.
- The manifest declares actions, event hooks, panes, widgets, buttons, CLI subcommands, build steps and prebuilt release binaries per OS.
- Panes run as backend sessions (tmux today, the Windows host later) and show as tiles or overlays, so plugin UI gets the same terminal behaviour as agent tiles.
- Widgets are declarative: a plugin pushes styled text segments and click targets to a named slot (rail row, tile title, strip) over the daemon API; the TUI draws them.
- New CLI: `claude-garage plugin install owner/repo`, `plugin link <path>`, `plugin update`, `plugin list`, `plugin remove`.
- Plugins declare permissions; install shows them and flags "sends input to sessions" as dangerous. There is no sandbox, and garage says so.
- Every action call gets a full context JSON (focused session, workspace, selection, worktree).
- Each plugin gets a storage directory. The plugin API is versioned by `api_version` with feature checks. A JSON schema for the manifest ships with garage.

## Capabilities

### New Capabilities
- `plugin-host`: manifest, install lifecycle, runtime (actions, events, panes, widgets, buttons, CLI), permissions, context, storage, API versioning.

## Impact

- **Code:** Rust daemon (from p21) gains a plugin registry, process runner and widget store; the TUI (seams from p22) renders widget slots and binds plugin actions; CLI gains `plugin` subcommands.
- **Depends on:** p21-rust-daemon (API, event bus, backend sessions) and p22-tui-seams (action registry, slot registry).
- **Docs:** manifest reference and plugin API reference.
- No change to existing user-visible behaviour when no plugins are installed.
