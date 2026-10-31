## MODIFIED Requirements

### Requirement: Prerequisite checks before starting
Before starting the daemon, the entrypoint SHALL verify that the prerequisites for the host operating system are available on `PATH`: `claude` (Claude Code CLI) on every OS, and `tmux` on macOS and Linux unless the host backend is selected. If one is missing, the entrypoint SHALL print an actionable error message naming the specific missing tool and SHALL exit with code 1 without starting the daemon.

#### Scenario: tmux missing
- **WHEN** `npx claude-garage` is run on macOS or Linux with no `tmux` binary on `PATH`
- **THEN** the process prints an error naming `tmux` as the missing prerequisite, exits with code 1, and no daemon is started (`GET /api/health` is not reachable)

#### Scenario: claude missing
- **WHEN** `npx claude-garage` is run on a machine with no `claude` binary on `PATH`
- **THEN** the process prints an error naming `claude` (Claude Code CLI) as the missing prerequisite, exits with code 1, and no daemon is started

#### Scenario: Windows does not need tmux
- **WHEN** `npx claude-garage` is run on Windows with `claude` on `PATH` and no `tmux`
- **THEN** the daemon starts and no prerequisite error is printed

### Requirement: TUI binary availability
The package SHALL provide a prebuilt binary for every supported target: macOS arm64 and x64, Linux x64 and arm64, Windows x64. On an unsupported target, the launcher SHALL compile from source when a Rust toolchain (`cargo`) is available. If the binary is unavailable and cannot be built, `claude-garage` SHALL print an actionable error naming what is missing (a prebuilt binary for this platform, or the Rust toolchain) and exit non-zero without starting a broken UI.

#### Scenario: Missing binary is actionable
- **WHEN** `claude-garage` runs on an unsupported platform with no `cargo` on PATH
- **THEN** the command prints an error explaining how to get the binary (install Rust or use a supported platform) and exits non-zero

#### Scenario: Prebuilt on Linux
- **WHEN** `npx claude-garage` runs on Linux x64
- **THEN** the prebuilt Linux binary is used and nothing is compiled
