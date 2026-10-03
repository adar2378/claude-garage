# packaging

## Purpose

The `npx claude-garage` entrypoint: one process serving UI + API on loopback, actionable prerequisite errors, and shutdown that never touches tmux sessions.
## Requirements
### Requirement: Prerequisite checks before starting
Before starting the daemon, the entrypoint SHALL verify that both `tmux` and `claude` (Claude Code CLI) are available on `PATH`. If either is missing, the entrypoint SHALL print an actionable error message naming the specific missing tool and SHALL exit with code 1 without starting the daemon.

#### Scenario: tmux missing
- **WHEN** `npx claude-garage` is run on a machine with no `tmux` binary on `PATH`
- **THEN** the process prints an error naming `tmux` as the missing prerequisite, exits with code 1, and no daemon is started (`GET /api/health` is not reachable)

#### Scenario: claude missing
- **WHEN** `npx claude-garage` is run on a machine with `tmux` installed but no `claude` binary on `PATH`
- **THEN** the process prints an error naming `claude` (Claude Code CLI) as the missing prerequisite, exits with code 1, and no daemon is started

### Requirement: Graceful shutdown never kills tmux sessions
On receiving SIGINT or SIGTERM, the process SHALL shut down its own HTTP/WebSocket server without sending any kill signal to tmux or terminating any garage tmux session. Sessions live at shutdown time SHALL remain live in tmux and SHALL be reattachable after the process is started again.

#### Scenario: SIGINT does not kill sessions
- **WHEN** the process is running with a live garage session `garage/kowboy/checkout` and receives SIGINT (e.g. Ctrl+C)
- **THEN** the process exits, and `tmux ls` (run independently) still shows `garage/kowboy/checkout` as a live session

#### Scenario: Sessions reachable after restart
- **WHEN** the process is stopped via SIGTERM while `garage/kowboy/checkout` is live, and `npx claude-garage` is run again
- **THEN** `GET /api/sessions` on the new process reports `garage/kowboy/checkout` as live (not restorable), and a WebSocket connection to `/term/garage%2Fkowboy%2Fcheckout` attaches to the same running tmux session

### Requirement: Port conflict produces a readable error
If the configured port (default 4747) is already in use, the entrypoint SHALL exit with a readable error message suggesting the `GARAGE_PORT` environment variable as the way to choose a different port, rather than an unhandled stack trace.

#### Scenario: Port already bound
- **WHEN** port 4747 is already occupied by another process and `npx claude-garage` is run without `GARAGE_PORT` set
- **THEN** the process prints a readable error mentioning the port conflict and the `GARAGE_PORT` environment variable, then exits (rather than crashing with a raw `EADDRINUSE` stack trace)

#### Scenario: GARAGE_PORT selects an alternate port
- **WHEN** `GARAGE_PORT=5050` is set in the environment and `npx claude-garage` is run while 4747 is occupied
- **THEN** the daemon starts successfully listening on `127.0.0.1:5050`

### Requirement: TUI subcommand
`npx claude-garage tui` SHALL launch the TUI client. It SHALL start the daemon first if `GET /api/health` is not reachable (same prerequisite checks and port as before), then run the TUI attached to it. Exiting the TUI SHALL leave the daemon and all tmux sessions running. `tui` SHALL remain accepted as an alias of the bare command.

#### Scenario: TUI starts daemon when absent
- **WHEN** `npx claude-garage tui` is run with no daemon listening on 4747
- **THEN** the daemon starts, then the TUI opens full-screen; quitting the TUI leaves `GET /api/health` reachable

#### Scenario: TUI reuses running daemon
- **WHEN** `npx claude-garage tui` is run while a daemon is already serving 4747
- **THEN** no second daemon is started and the TUI attaches to the existing one

### Requirement: TUI binary availability
The package SHALL provide the compiled TUI binary for the host platform (macOS arm64 at minimum), either shipped per-release or compiled on first run when a Rust toolchain (`cargo`) is available. The build SHALL be `npm run build:tui` invoking cargo on the `wall/` workspace, producing the binary at the launcher's platform-specific dist path. If the binary is unavailable and cannot be built, `claude-garage tui` SHALL print an actionable error naming what is missing (prebuilt binary for this platform, or install the Rust toolchain) and exit non-zero without starting a broken UI. During the port's transition window the launcher MAY fall back to the Dart binary; after parity removal, the Rust binary is the only TUI.

#### Scenario: Missing binary is actionable
- **WHEN** `claude-garage tui` runs on a platform with no prebuilt binary and no `cargo` on PATH
- **THEN** the command prints an error explaining how to get the TUI (install Rust or use a supported platform) and exits non-zero

#### Scenario: Self-build via cargo
- **WHEN** no prebuilt binary exists but `cargo` is on PATH
- **THEN** the launcher builds the `wall/` workspace once (announcing it), uses the result, and subsequent launches reuse it

### Requirement: Bare entrypoint runs the TUI
Running `npx claude-garage` with no subcommand SHALL behave exactly like `npx claude-garage tui`. The daemon SHALL NOT serve any UI files and SHALL NOT accept WebSocket connections under `/term`.

#### Scenario: Bare command opens the TUI
- **WHEN** a user runs `npx claude-garage` with no daemon running
- **THEN** the daemon starts detached and the TUI opens full-screen; no browser is opened

#### Scenario: No UI is served
- **WHEN** the daemon is running and `GET http://127.0.0.1:4747/` is requested
- **THEN** no HTML page is returned, while `GET /api/health` returns 200

