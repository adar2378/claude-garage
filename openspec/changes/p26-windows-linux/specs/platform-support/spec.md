## ADDED Requirements

### Requirement: Supported operating systems
garage SHALL run natively on macOS (arm64, x64), Linux (x64, arm64) and Windows 10 1809+ / 11 (x64). Each supported target SHALL have a prebuilt release binary and SHALL pass the CI test matrix.

#### Scenario: Install on each OS
- **WHEN** a user on macOS, Linux or Windows runs `npx claude-garage`
- **THEN** the binary for their platform is used without compiling, and the wall opens

### Requirement: Session backend per OS
The daemon SHALL own sessions through a session backend selected once at start: the tmux backend on macOS and Linux, the host backend on Windows. `GARAGE_BACKEND=host` SHALL select the host backend on any OS. Shared code SHALL NOT branch on the operating system for session operations.

#### Scenario: Windows uses the host backend
- **WHEN** the daemon starts on Windows
- **THEN** new sessions run in per-session host processes and no `tmux` binary is required

#### Scenario: Forced host backend on macOS
- **WHEN** the daemon starts on macOS with `GARAGE_BACKEND=host`
- **THEN** new sessions run in host processes and the tmux server is not used

### Requirement: Host sessions outlive the daemon and terminal
On the host backend, a session SHALL keep running when the daemon stops, the TUI quits, or the terminal window closes. After a reboot, sessions that died SHALL appear as restorable, as with tmux.

#### Scenario: Daemon restart keeps sessions
- **WHEN** a host-backed session is working and the daemon is restarted
- **THEN** the session is still live afterwards and the TUI reattaches to the same screen and scrollback

#### Scenario: Reboot makes sessions restorable
- **WHEN** the machine reboots while host-backed sessions were live
- **THEN** the next start lists those sessions as restorable

### Requirement: Platform features on every OS
Desktop notifications, the folder picker and the clipboard SHALL work on macOS, Linux and Windows through one OS module. Where a desktop facility is unavailable (for example no notification daemon on a headless Linux), the feature SHALL degrade to a no-op with a log line, not an error.

#### Scenario: Notification on Linux
- **WHEN** a session on Linux transitions to needs-input with a notification daemon running
- **THEN** a desktop notification is shown

#### Scenario: Headless Linux
- **WHEN** the same transition happens on a Linux machine with no notification daemon
- **THEN** no error is shown in the wall and the transition is logged

### Requirement: Modified Enter keys through ConPTY
On Windows, Shift+Enter in an engaged tile SHALL insert a newline in Claude Code's prompt, and Ctrl+Enter SHALL send a queued message immediately.

#### Scenario: Shift+Enter on Windows
- **WHEN** the user presses Shift+Enter while typing in an engaged tile on Windows
- **THEN** Claude Code's prompt gains a new line and nothing is submitted
