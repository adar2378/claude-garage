## Why

The wall looks frozen at the moments it is busy: the daemon starting, a tile attaching, sessions restoring, the daemon restarting. A `done` session also drops its highlight in one hard step. Small, purposeful motion shows that something is happening and makes state changes easier to notice, without pulling attention from blocked sessions.

## What Changes

- Startup: a short wordmark reveal, shown only while the daemon is really starting. If the daemon is already up, the wall opens straight away.
- Spinners on tiles while they attach or restore, on the strip during a daemon restart, and during plugin install (once p24 lands).
- `done` fades out over its 2 minutes instead of switching off.
- New tiles fade in; closed sessions dissolve.
- Rules: no redraw timer when nothing animates; `needs-input` amber stays the only loud signal; `animations = false` in config and a no-motion environment variable turn motion off; motion never delays a keystroke.
- Crates: `tachyonfx` (effects), `throbber-widgets-tui` (spinners), `tui-big-text` (wordmark).

## Capabilities

### New Capabilities
- `tui-motion`: when the wall animates, how it stays cheap, and how to turn it off.

## Impact

- **Code:** `wall/Cargo.toml` (three crates), a motion module in `wall/src/ui/`, the render loop's tick scheduling, startup path, tile and strip rendering, config loading.
- **Depends on:** p22-tui-seams (render loop and slots carved out of `runtime.rs`).
- No daemon or API change.
