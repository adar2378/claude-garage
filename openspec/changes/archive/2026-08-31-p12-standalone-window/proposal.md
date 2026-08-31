# Proposal: p12-standalone-window

## Why

`m` maximizes a tile inside the wall, but sometimes you want the focused
session in its own OS window — full native scrollback, its own space/desktop,
copy-paste via the terminal app you already use. tmux already multiplexes
multiple attach clients onto one session, so this is purely additive: the
wall keeps its own client, a second one opens alongside it.

## What Changes

- New garage-layer binding `t`: launches a detached OS terminal window
  (iTerm2 if installed, else Terminal.app) running
  `tmux attach -t '=<session-id>'` for the focused LIVE session. The wall's
  own tile keeps rendering — tmux serves both clients.
- Declined by the store like `Spawn`/`Close` (an effect, not a state
  transition); the router shows "no live session to open" for a
  restorable/dead focus, "standalone windows: macOS only for now" off
  macOS, and success/failure strip notices otherwise.
- Help overlay (`?`) lists the new binding.

## Capabilities

### New Capabilities
_None — this extends `tui-key-routing`'s existing garage-layer binding list
with one more single-key command; no new capability doc._

### Modified Capabilities
_None formally tracked; see tasks.md — small enough to fold into the existing
tui-key-routing behavior without a spec delta._

## Impact

- `wall/src/state/store.rs`: `GarageCommand::OpenWindow`, `"t"` mapping,
  declined dispatch.
- `wall/src/runtime.rs`: `Effect::OpenWindow`, router decline handling,
  effect execution (spawn + detach, never blocking the state loop).
- `wall/src/ui/window_open.rs` (new): argv construction for iTerm2/Terminal.app
  as pure functions (unit-testable without a GUI), shell/AppleScript escaping
  helper, the actual detached launch.
- `wall/src/ui/help.rs`: one more legend row.
- No daemon, web UI, or persisted-schema changes.
