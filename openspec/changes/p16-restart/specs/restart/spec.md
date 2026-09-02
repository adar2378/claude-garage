# restart

## ADDED Requirements

### Requirement: Restart the daemon from the terminal
`claude-garage restart` SHALL stop the running daemon and start a fresh detached one on the same port, wait until health answers with the launcher's version, and print it. tmux sessions SHALL be untouched. If no daemon is running it SHALL just start one.

#### Scenario: Running TUI survives a daemon restart
- **WHEN** a TUI is attached and `claude-garage restart` runs in another terminal
- **THEN** the TUI shows the daemon as boxed, then live, with the same tiles and no user action

### Requirement: Daemon self-restart endpoint
`POST /api/daemon/restart` SHALL spawn a detached successor daemon with the same environment, reply 202 with the successor pid, then close and exit. The successor SHALL wait for the predecessor's port to free before listening and SHALL fail loud within 10s if it cannot.

#### Scenario: Handoff without a port race
- **WHEN** the endpoint is called
- **THEN** health stops answering for the old pid and answers for the new pid within 5s, and no request sees a second daemon on the port

### Requirement: Restart sessions in place
`POST /api/sessions/restart` with `{id}` or `{all: true, force?}` SHALL respawn each target's tmux pane with `claude --resume <claudeSessionId>` in the session's directory (worktree dir for worktree sessions), keeping the tmux session name. Sessions in `working` or `needs-input` SHALL be skipped unless `force`. The response SHALL list `restarted` (with `resumed: true|false`), `skipped` (with status) and `failed` (with the error). A target without a known Claude session id SHALL restart as plain `claude` and be reported with `resumed: false`.

#### Scenario: Idle session picks up the new binary
- **WHEN** `claude` was upgraded and an idle session is restarted
- **THEN** the same tmux session runs the new version, the conversation is resumed, and the wall's tile keeps its title and position

#### Scenario: Busy session is skipped
- **WHEN** `{all: true}` runs while one session is `working`
- **THEN** that session appears in `skipped` with status `working` and keeps running

### Requirement: CLI session restart
`claude-garage restart --sessions` SHALL restart the daemon, then call the session endpoint with `all: true` and `force` equal to `--all`, and print one line per restarted, skipped and failed session.

#### Scenario: Skips are visible
- **WHEN** two sessions are idle and one is working, without `--all`
- **THEN** the output lists two restarted and one skipped with its status

### Requirement: TUI chords
In the garage layer, `r r` SHALL restart the focused session with an armed double-press (the arming notice SHALL say it resumes the conversation, and SHALL warn when the session is busy; the second press forces), `r a` SHALL restart every idle/done session in the focused workspace, `r d` SHALL restart the daemon. Each SHALL show a strip notice with the outcome. `?` SHALL list all three.

#### Scenario: Armed restart of a busy session
- **WHEN** the focused session is `working` and the user presses `r r`
- **THEN** the strip warns it is working and asks for `r r` again; a second `r r` within 3s restarts it

### Requirement: Web wall restart control
The session cell's hover controls SHALL include a restart action that calls the session endpoint for that session and reflects `skipped`/`failed` in the existing error surface.
