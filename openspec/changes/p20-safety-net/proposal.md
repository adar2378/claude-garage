## Why

CI only runs when a version tag is pushed, and only on macOS. A pull request gets no checks at all, so a contributor cannot see whether their change builds on Linux or Windows, and we cannot tell either. The coming refactor (Rust daemon in p21, TUI seams in p22) rewrites code the user runs every day. It needs a safety net first: checks on every PR, on all three operating systems, plus tests that pin today's behaviour.

The daemon tests also start and kill sessions on the real tmux server. That is unsafe on a developer machine with live sessions.

## What Changes

- A new CI workflow runs on every pull request and every push to `main`, on `macos-14`, `ubuntu-latest` and `windows-latest`.
  - Rust: `cargo test --lib --bins` and `cargo clippy --all-targets -- -D warnings`.
  - Daemon: `npm test`, with tmux installed (Homebrew on macOS, apt on Ubuntu). Skipped on Windows, which has no tmux.
  - Windows jobs are `continue-on-error` at first. They show the gap without blocking merges.
- `GARAGE_TMUX_SOCKET` in the daemon: when set, every tmux call uses `tmux -L <socket>`, a separate tmux server. Tests always set it, so they never touch the user's sessions.
- TUI snapshot tests (insta + ratatui `TestBackend`) for the rail, the strip and the tile title.
- An API contract test suite. It starts a daemon on an isolated port, `GARAGE_DIR` and tmux socket, and drives it over HTTP only. The same suite will run against the Rust daemon in p21.

## Capabilities

### New Capabilities
- `ci-quality-gates`: what runs on every PR, on which systems, and how tests stay isolated from the user's tmux server.

## Impact

- **Code:** `.github/workflows/ci.yml` (new), `daemon/src/tmux.js` and the two direct `tmux` calls in `daemon/src/poller.js` and `daemon/src/session-resolve.js` (socket knob), `daemon/test/*` (set the socket), `wall/Cargo.toml` (insta as a dev-dependency), new snapshot tests under `wall/src/ui/`, new contract tests under `tests/contract/`.
- **Release:** unchanged. `release.yml` keeps running on tags.
- No user-visible change.
