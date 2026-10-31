## MODIFIED Requirements

### Requirement: TUI binary availability
The package SHALL ship one native `claude-garage` binary per supported platform (macOS arm64 and x64, Linux x64 and arm64 at minimum), containing the TUI, the daemon and the CLI. Installing through npm (`npx claude-garage`), Homebrew or the shell installer SHALL fetch the binary for the host platform; no Node runtime and no Rust toolchain SHALL be needed to run it. On a platform with no published binary, the installer SHALL print an actionable error naming the platform and exit non-zero without installing a broken command.

#### Scenario: npx on a supported platform
- **WHEN** a user on Linux x64 with no Rust toolchain runs `npx claude-garage`
- **THEN** npm installs the Linux x64 binary package and the TUI opens, with the daemon started from the same binary

#### Scenario: Unsupported platform is actionable
- **WHEN** `npx claude-garage` runs on a platform with no published binary
- **THEN** the command prints an error naming the platform and the supported ones, and exits non-zero
