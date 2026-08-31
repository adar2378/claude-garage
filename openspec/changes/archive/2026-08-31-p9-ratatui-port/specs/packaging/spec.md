# packaging (delta)

## MODIFIED Requirements

### Requirement: TUI binary availability
The package SHALL provide the compiled TUI binary for the host platform (macOS arm64 at minimum), either shipped per-release or compiled on first run when a Rust toolchain (`cargo`) is available. The build SHALL be `npm run build:tui` invoking cargo on the `wall/` workspace, producing the binary at the launcher's platform-specific dist path. If the binary is unavailable and cannot be built, `claude-garage tui` SHALL print an actionable error naming what is missing (prebuilt binary for this platform, or install the Rust toolchain) and exit non-zero without starting a broken UI. During the port's transition window the launcher MAY fall back to the Dart binary; after parity removal, the Rust binary is the only TUI.

#### Scenario: Missing binary is actionable
- **WHEN** `claude-garage tui` runs on a platform with no prebuilt binary and no `cargo` on PATH
- **THEN** the command prints an error explaining how to get the TUI (install Rust or use a supported platform) and exits non-zero

#### Scenario: Self-build via cargo
- **WHEN** no prebuilt binary exists but `cargo` is on PATH
- **THEN** the launcher builds the `wall/` workspace once (announcing it), uses the result, and subsequent launches reuse it
