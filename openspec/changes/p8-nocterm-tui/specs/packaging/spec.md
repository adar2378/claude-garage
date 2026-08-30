# packaging (delta)

## ADDED Requirements

### Requirement: TUI subcommand
`npx claude-garage tui` SHALL launch the TUI client. It SHALL start the daemon first if `GET /api/health` is not reachable (same prerequisite checks and port as the web entrypoint), then run the TUI attached to it. Exiting the TUI SHALL leave the daemon and all tmux sessions running. The bare `npx claude-garage` behavior SHALL be unchanged.

#### Scenario: TUI starts daemon when absent
- **WHEN** `npx claude-garage tui` is run with no daemon listening on 4747
- **THEN** the daemon starts, then the TUI opens full-screen; quitting the TUI leaves `GET /api/health` reachable

#### Scenario: TUI reuses running daemon
- **WHEN** `npx claude-garage tui` is run while a daemon is already serving 4747
- **THEN** no second daemon is started and the TUI attaches to the existing one

### Requirement: TUI binary availability
The package SHALL provide the compiled TUI binary for the host platform (macOS arm64 at minimum), either shipped per-release or compiled on first run when a Dart SDK is available. If the binary is unavailable and cannot be built, `claude-garage tui` SHALL print an actionable error naming what is missing and exit non-zero without starting a broken UI.

#### Scenario: Missing binary is actionable
- **WHEN** `claude-garage tui` runs on a platform with no prebuilt binary and no Dart SDK
- **THEN** the command prints an error explaining how to get the TUI (install Dart or use a supported platform) and exits non-zero
