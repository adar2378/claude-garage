## Why

The plugin host (p24) is only proven if real features run on it. The open contributor PR #2 bundles a set of useful non-Windows features into shared core files. Rebuilding those features as first-party plugins is the acceptance test for the plugin API: if they work as plugins, the API is good enough to freeze as v1. Features that are terminal fundamentals stay in the core, because forcing them through a plugin API would make the API worse.

## What Changes

- Ship these as first-party plugins in the repo (`plugins/`), one plugin each:
  - **closed-chats**: closed sessions stay grey in the rail; `H` opens a recent list.
  - **jump-palette**: `/` opens a palette to jump to any session.
  - **quick-send**: `s` / `S` send a line to the focused or chosen session.
  - **tile-buttons**: close and delete buttons on tiles.
  - **usage-bar**: 5-hour and weekly usage bar in the strip, persisted across restarts.
  - **turn-markers**: markers for each agent turn in the scrollback.
  - **prune-empty-projects**: closing a project's last chat removes the project.
  - **cli-extras**: `claude-garage <folder>`, `claude-garage install`, `claude-garage prune`.
  - **pit-pet**: the existing pit pet, moved out of the core.
- Build these in the core, not as plugins: draft and pasted-screenshot recovery, mouse selection and copy, sidebar hide and resize, quiet startup.
- Freeze plugin API v1 only after every plugin above works. Any API gap found here is fixed in the plugin host first.

## Feature to capability map

| Feature | What the plugin needs | Where it lives |
|---|---|---|
| Closed chats grey in the rail, `H` recent list | rail-row widget slot, overlay pane; daemon keeps closed-session records | core data + plugin UI |
| `/` jump palette | overlay pane + `session.focus` action | plugin |
| Quick send `s` / `S` | action + key + send-text-to-session API (`sessions.input`) | plugin |
| Close / delete buttons | `tile.title.right` button slot calling `session.close` / `session.delete` | plugin |
| 5h / weekly usage bar | `strip.right` widget slot, statusline events, plugin storage | plugin |
| Turn markers | scrollback marker slot + transcript events | plugin |
| Closing the last chat removes the project | `session.closed` event + remove-workspace API | plugin |
| `claude-garage <folder>` / `install` / `prune` | `[[cli]]` subcommands | plugin |
| Pit pet | strip widget slot, status events, click target | plugin |
| Draft + screenshot recovery | reads everything typed into a session | core (too risky for an unsandboxed plugin) |
| Mouse selection / copy, sidebar hide / resize, quiet startup | terminal and layout internals | core |

## Capabilities

### New Capabilities
- `first-party-plugins`: the plugins listed above and the API v1 freeze rule.

### Modified Capabilities
- `tui-wall`: adds draft and screenshot recovery, mouse selection and copy, sidebar hide and resize.
- `packaging`: adds quiet startup.

## Impact

- **Code:** new `plugins/` folder; daemon keeps closed-session records; core TUI work for the four core items; `ui/pet.rs` moves into a plugin.
- **Depends on:** p24-plugin-host.
- **Behaviour:** with the first-party plugins enabled by default, users see the same pit pet as today; the other features are new.
