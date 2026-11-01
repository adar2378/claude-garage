## Why

The TUI works, but its behaviour is wired straight into two large files: `wall/src/runtime.rs` (4,549 lines) and `wall/src/state/store.rs` (2,457 lines). Keys map to code through hard-coded `match` arms (`garage_command_for`, `restart_command_for`, `handle_overlay_key`, `handle_click`). There is no named list of what the wall can do, and no defined place where a new UI element can go.

That blocks three planned phases. p23-motion needs clear points to start effects. p24-plugin-host needs actions that plugins can call and slots where plugins can draw. A user keymap needs actions to bind to. Today each of these would mean editing the two big files again.

## What Changes

- **Action registry.** Every wall behaviour becomes a named action with a stable id (for example `session.close`, `session.restart`, `session.focus`, `jump.blocked`, `workspace.switch`, `overlay.open`). An action runs with a context payload: focused session, focused workspace, current selection, worktree record. The existing `GarageCommand` enum is the starting point.
- **Keymap layer.** Garage-layer keys, chords (`r r`, `r a`, `r d`, `X` then `K`) and overlay keys resolve through one keymap table of key → action id. Defaults reproduce today's bindings exactly. A user config file comes later (not in this change).
- **Mouse through actions.** Clicks on tiles, rail rows, the triage queue and the view picker resolve to the same action ids as keys.
- **Slot registry.** Named UI contribution points, rendered by core: `tile.title.right`, `rail.row`, `rail.header`, `strip.right`, `overlay`, `context.menu`. In this change only core widgets use them (for example the context meter in the tile title and the usage chip in the strip). Plugins use them from p24.
- **Split the big files** along these seams into modules: actions, keymap, slots, effects, event loop, and per-overlay handlers.
- **Pit pet behind a boundary.** `ui/pet.rs` and its runtime wiring move behind one module interface and a cargo feature (`pet`, on by default), so p25 can turn the pet into a plugin.
- **No visible behaviour change.** Same keys, same clicks, same rendering, same strip wording.

## Capabilities

### New Capabilities
- `tui-actions`: named actions with a context payload, the keymap layer, mouse-to-action routing, and the slot registry.

### Modified Capabilities
- None. `tui-key-routing` requirements stay true as written: this change moves how they are implemented, not what they say.

## Impact

- **Code:** `wall/src/runtime.rs`, `wall/src/state/store.rs`, `wall/src/ui/{tile,rail,strip,hit_targets,pet}.rs`, new modules under `wall/src/actions/`, `wall/src/keymap.rs`, `wall/src/slots/`, and `wall/src/runtime/` (split). `wall/Cargo.toml` gains a `pet` feature.
- **Depends on:** p20-safety-net (insta snapshot tests and CI on three OSes guard the refactor).
- **Enables:** p23-motion (effects start from actions and slots), p24-plugin-host (plugins call actions and fill slots), and the user keymap config.
- No daemon change, no API change, no release on its own.
