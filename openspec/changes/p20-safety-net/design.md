## Context

`release.yml` runs tests only on `v*` tags, on `macos-14`. Daemon tests (`sessions.title.test.js`, `sessions.context.test.js`) create and kill `garage/<run>/…` sessions on whatever tmux server is default. On a developer machine that is the user's live server. `wall/tests/injection.rs` does the same, which is why the documented command is `cargo test --lib --bins`.

The daemon calls tmux in three places: `daemon/src/tmux.js`, plus direct calls in `poller.js` and `session-resolve.js`.

## Goals / Non-Goals

**Goals:** checks on every PR on three systems; tests that can never touch the user's tmux server; a frozen record of today's API and today's rail before the rewrite.

**Non-Goals:** making Windows pass (p26); moving releases to cargo-dist (p21); end-to-end tests with a real Claude Code session; coverage targets.

## Decisions

**1. One `ci.yml`, a matrix over three systems.** Jobs: `rust` (test + clippy), `daemon` (npm test), `contract`. Windows runs `rust` only and is `continue-on-error: true`. Alternative: separate workflows per system. Rejected: one matrix is easier to read and to make required later.

**2. `GARAGE_TMUX_SOCKET` wraps every tmux call.** A single helper in `tmux.js` (`tmuxArgs(args)`) prepends `-L <socket>` when the variable is set. `poller.js` and `session-resolve.js` go through it. Unset means today's behaviour, so the user's daemon is unchanged. Test files set a unique socket per run and kill that server (`tmux -L <socket> kill-server`) when done.

**3. Snapshot tests render real widgets into `TestBackend`.** Fixtures build `WorkspaceGroup`s and sessions, render the rail, strip and tile title at a fixed size, and compare against `insta` snapshots. Snapshots are reviewed like code. They catch accidental visual changes during the p22 carve-out.

**4. Contract tests talk HTTP only.** A small Node runner (`tests/contract/`) starts the daemon binary given by `GARAGE_DAEMON_CMD` (default: the JS daemon) with a free port, a temp `GARAGE_DIR`, a unique tmux socket and `GARAGE_CLAUDE_CMD=sh`. It checks status codes, JSON shapes and the `/api/events` stream for each route. It never imports daemon code, so p21 points `GARAGE_DAEMON_CMD` at the Rust binary and reuses it unchanged.

**5. Clippy with `-D warnings` on CI.** Today clippy is clean, so this keeps it clean.

## Risks / Trade-offs

- macOS runners are slower to start. Accepted: macOS is the main user platform.
- `apt install tmux` gives an older tmux on Ubuntu. Tests use only basic commands, so this is acceptable; a failure there is a real Linux bug.
- Snapshot tests can become noise if updated without looking. Mitigation: CI fails on pending snapshots, and reviewers check the `.snap` diff.
