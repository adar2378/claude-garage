## Why

garage ships two runtimes: an 18k-line Rust TUI and a 3,200-line Node daemon behind a 556-line Node launcher (`bin/garage.js`). Users need Node 20+ and a Rust binary. Session and status shapes are defined twice (JS and Rust) and drift. The daemon's platform code is macOS-only (`osascript` picker and notifications), so Linux and Windows support would mean more JS workarounds.

The refactor plan (seams for session backends, an OS layer, an event bus, then a plugin host) rewrites most of the daemon anyway. Doing it once, in Rust, gives one binary on three operating systems, one language for contributors, and shared types.

## What Changes

- New root Cargo workspace: `wall` (unchanged location), `crates/protocol` (shared API types), `crates/daemon` (the daemon as a library), `crates/cli` (the single `claude-garage` binary).
- The daemon is rewritten in Rust (tokio, axum, tower-http). The HTTP API stays the same, route for route, so the current TUI works against either daemon unchanged.
- New capability spec `daemon-api`: every route, request, response and live event, written down. It is the source of truth for the contract tests.
- A `SessionBackend` trait (create, kill, list, capture, respawn, rename, attach command) with tmux as the only implementation. The Windows backend comes in p26 behind the same trait.
- An OS module (notifications, folder picker, open window, clipboard) built on cross-platform crates (`notify-rust`, `rfd`, `arboard`), replacing the macOS-only `osascript` code.
- An in-process event bus replacing the two Node `EventEmitter`s (`statusEvents`, `pollerEvents`).
- tmux socket isolation from the first commit: the daemon takes a tmux socket name (`GARAGE_TMUX_SOCKET`, passed as `tmux -L`), so tests never touch the user's real tmux server.
- After parity: tmux control mode (`tmux -C`) replaces the 2-second polling loop, as its own task.
- **BREAKING (internal):** `GET /api/diff/:workspace` is removed. The TUI has not used it since 0.5.0, when diff review left with the web wall.
- One `claude-garage` binary with `clap` subcommands (`tui`, `daemon`, `restart`, `plugin` reserved for p24) replaces `bin/garage.js`. Distribution moves to `cargo-dist`: per-OS npm packages so `npx claude-garage` keeps working, plus Homebrew and a shell installer.

## Capabilities

### New Capabilities
- `daemon-api`: the daemon's HTTP and live-event contract.

### Modified Capabilities
- `packaging`: "TUI binary availability" becomes one native binary per platform, no Node or cargo needed at runtime.

### Removed Capabilities
- `workspace-diff`: the diff endpoint is removed.

## Impact

- **Code:** new `Cargo.toml` (workspace root), `crates/protocol`, `crates/daemon`, `crates/cli`; `wall/Cargo.toml` loses its standalone `[workspace]` table and uses `crates/protocol` types. `daemon/` and `bin/garage.js` are deleted at cutover, not before.
- **Release:** `.github/workflows/release.yml` and `package.json` change at cutover (cargo-dist).
- **Users:** no Node needed after cutover. The daemon restarts once on upgrade (version gate); tmux sessions survive.
- **No change** to the TUI's behaviour, `~/.garage/state.json` format, hook or statusline install format, or the tmux session naming (`garage/<workspace>/<label>`).
