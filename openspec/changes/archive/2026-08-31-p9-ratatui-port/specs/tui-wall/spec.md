# tui-wall (delta)

## MODIFIED Requirements

### Requirement: Performance under load
The TUI SHALL remain responsive under output load: with 6 tiles receiving a combined 200+ lines/second, a keypress SHALL be processed within 50 ms and process CPU SHALL stay under 15% of one core. The TUI SHALL disable terminal flow control (`IXON`) and signal generation (`ISIG`) for its own tty at startup so Ctrl+Q/Ctrl+S/Ctrl+C reach the application, and SHALL restore all terminal settings on exit. No specific frame-rate cap is mandated.

#### Scenario: Input latency under load
- **WHEN** 5 tiles stream heavy colored output and the user presses a workspace digit
- **THEN** the focus change is processed within 50 ms of the keypress reaching the process

#### Scenario: Ctrl+Q reaches the app
- **WHEN** the user presses Ctrl+Q after startup
- **THEN** the application receives the key event (the tty driver does not consume it)

## ADDED Requirements

### Requirement: Teardown never injects into panes
Closing a tile's PTY, quitting the TUI, or the TUI dying SHALL never inject bytes into the attached tmux pane. Attach clients SHALL be detached via `tmux detach-client` before their PTY master is closed; process-kill paths that would deliver SIGHUP/EOF to an attach client SHALL be preceded by the same detach. An idle interactive shell in an attached session SHALL survive any TUI exit.

#### Scenario: Quit leaves an idle shell alive
- **WHEN** a tile is attached to a session running an idle interactive shell and the user quits the TUI
- **THEN** the session still exists and the shell has received no input bytes (verifiable with a recorder pane)

#### Scenario: SIGKILL of the wall injects nothing
- **WHEN** the TUI process is killed with SIGKILL while tiles are attached
- **THEN** attached sessions survive and no `\n`/EOF byte reaches any pane from the dying clients
