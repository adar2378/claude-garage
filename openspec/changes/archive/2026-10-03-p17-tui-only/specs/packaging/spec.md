## ADDED Requirements

### Requirement: Bare entrypoint runs the TUI
Running `npx claude-garage` with no subcommand SHALL behave exactly like `npx claude-garage tui`. The daemon SHALL NOT serve any UI files and SHALL NOT accept WebSocket connections under `/term`.

#### Scenario: Bare command opens the TUI
- **WHEN** a user runs `npx claude-garage` with no daemon running
- **THEN** the daemon starts detached and the TUI opens full-screen; no browser is opened

#### Scenario: No UI is served
- **WHEN** the daemon is running and `GET http://127.0.0.1:4747/` is requested
- **THEN** no HTML page is returned, while `GET /api/health` returns 200

## MODIFIED Requirements

### Requirement: TUI subcommand
`npx claude-garage tui` SHALL launch the TUI client. It SHALL start the daemon first if `GET /api/health` is not reachable (same prerequisite checks and port as before), then run the TUI attached to it. Exiting the TUI SHALL leave the daemon and all tmux sessions running. `tui` SHALL remain accepted as an alias of the bare command.

#### Scenario: TUI starts daemon when absent
- **WHEN** `npx claude-garage tui` is run with no daemon listening on 4747
- **THEN** the daemon starts, then the TUI opens full-screen; quitting the TUI leaves `GET /api/health` reachable

#### Scenario: TUI reuses running daemon
- **WHEN** `npx claude-garage tui` is run while a daemon is already serving 4747
- **THEN** no second daemon is started and the TUI attaches to the existing one

## REMOVED Requirements

### Requirement: Single-process entrypoint serving UI and API
**Reason**: The web UI and `/term` WebSockets are removed (p17-tui-only).
**Migration**: `npx claude-garage` now runs the TUI; see "Bare entrypoint runs the TUI".

### Requirement: Daemon origin accepted by the Origin allowlist
**Reason**: No UI is served from the daemon's origin any more, so no browser requests come from it.
**Migration**: None needed; the TUI does not send an Origin header.
