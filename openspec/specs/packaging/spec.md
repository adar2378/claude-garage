# packaging

## Purpose

The `npx claude-garage` entrypoint: one process serving UI + API on loopback, actionable prerequisite errors, and shutdown that never touches tmux sessions.

## Requirements

### Requirement: Single-process entrypoint serving UI and API
Running `npx claude-garage` (via the package's `bin` entrypoint) SHALL start a single process that serves both the daemon API and the pre-built UI, listening on `127.0.0.1:4747`. The UI SHALL be reachable at `http://127.0.0.1:4747`, the API SHALL be reachable under `/api`, and terminal WebSocket connections SHALL be reachable under `/term`.

#### Scenario: Single command serves both UI and API
- **WHEN** a user runs `npx claude-garage`
- **THEN** a single process starts, `GET http://127.0.0.1:4747/` returns the built UI's HTML, `GET http://127.0.0.1:4747/api/health` returns 200, and a WebSocket connection to `ws://127.0.0.1:4747/term/<id>` is accepted for a live session

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

### Requirement: Daemon origin accepted by the Origin allowlist
When the daemon serves the UI from its own origin (`http://127.0.0.1:4747`, or the port selected via `GARAGE_PORT`), that origin SHALL be included in the Origin allowlist used for foreign-origin rejection, so that browser requests made from the served UI are not rejected as foreign.

#### Scenario: UI served from the daemon's own origin is not rejected
- **WHEN** the UI is loaded from `http://127.0.0.1:4747` and it issues `POST /api/sessions` with `Origin: http://127.0.0.1:4747`
- **THEN** the request is processed normally and is not rejected with 403

#### Scenario: Allowlist follows a custom port
- **WHEN** `GARAGE_PORT=5050` is set and the UI is served at `http://127.0.0.1:5050`
- **THEN** requests with `Origin: http://127.0.0.1:5050` are accepted, consistent with the daemon's own serving origin
