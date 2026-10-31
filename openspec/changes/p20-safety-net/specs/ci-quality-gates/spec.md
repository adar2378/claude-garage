## ADDED Requirements

### Requirement: Checks on every pull request
Every pull request and every push to `main` SHALL run the Rust tests (`cargo test --lib --bins`), clippy with warnings denied, the daemon tests and the API contract tests on macOS and Ubuntu. The Rust job SHALL also run on Windows, where a failure SHALL be reported but SHALL NOT block the merge until Windows support lands.

#### Scenario: A pull request gets checked
- **WHEN** a contributor opens a pull request
- **THEN** CI runs the Rust, daemon and contract jobs on macOS and Ubuntu, and the Rust job on Windows, and the results show on the pull request

#### Scenario: Windows failure does not block
- **WHEN** the Windows Rust job fails and every macOS and Ubuntu job passes
- **THEN** the pull request shows the Windows failure and is still mergeable

### Requirement: Tests never touch the user's tmux server
The daemon SHALL send every tmux command to the server named by `GARAGE_TMUX_SOCKET` (as `tmux -L <name>`) when that variable is set, and to the default server otherwise. Every test that starts tmux sessions SHALL set a unique socket and SHALL stop that server when it finishes.

#### Scenario: Local test run with live sessions
- **WHEN** a developer runs the daemon tests while their own garage sessions are running
- **THEN** `tmux ls` on the default server lists the same sessions before and after the run

#### Scenario: Unset means unchanged
- **WHEN** `GARAGE_TMUX_SOCKET` is not set
- **THEN** the daemon calls tmux exactly as before, with no `-L` argument

### Requirement: API contract suite
An HTTP-only test suite SHALL start a daemon from a configurable command on an isolated port, garage directory and tmux socket, and SHALL check every daemon route and the event stream. The suite SHALL NOT import daemon code, so it can run unchanged against another daemon implementation.

#### Scenario: Same suite, other daemon
- **WHEN** `GARAGE_DAEMON_CMD` points at a different daemon binary that serves the same API
- **THEN** the contract suite runs against it with no changes to the tests

### Requirement: TUI snapshot tests
The rail, the strip and the tile title SHALL have snapshot tests rendered into an in-memory terminal. CI SHALL fail when a snapshot changes without being accepted.

#### Scenario: Accidental visual change
- **WHEN** a refactor changes how the rail renders a family label
- **THEN** the snapshot test fails and shows the old and new rendering
