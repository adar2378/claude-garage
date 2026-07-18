# terminal-bridge

## Purpose

The WebSocket ⇄ node-pty ⇄ `tmux attach` pipeline that delivers an interactive terminal in the browser. Sessions survive any client or daemon death; the plain-terminal escape hatch is preserved.

## Requirements

### Requirement: Interactive terminal in the browser
The daemon SHALL expose `WS /term/:id` which spawns a `node-pty` process running `tmux attach -t <id>` and pipes bytes bidirectionally: pty output to the client as binary WebSocket frames, client binary frames to the pty as input. The UI SHALL render this stream in xterm.js such that a user can interact with the running Claude Code session in real time.

#### Scenario: User converses with Claude from the browser
- **WHEN** the UI is connected to `WS /term/garage/garage-dev/main` and the user types a prompt into the xterm.js terminal and presses Enter
- **THEN** the keystrokes appear in the terminal as typed and Claude Code's streamed response renders in the same terminal

#### Scenario: Control frames resize the pty
- **WHEN** the client sends a text frame `{"type":"resize","cols":120,"rows":40}`
- **THEN** the daemon resizes that connection's pty to 120×40 and the terminal reflows

### Requirement: WebSocket upgrades validate Origin
WebSockets bypass CORS, so the upgrade handler SHALL destroy any connection whose `Origin` header is present but outside the UI allowlist, before attaching a pty. Connections without an `Origin` header SHALL be allowed.

#### Scenario: Cross-site WebSocket hijack blocked
- **WHEN** a WebSocket upgrade for `/term/garage/...` arrives with `Origin: http://evil.example`
- **THEN** the socket is destroyed and no `tmux attach` pty is spawned

### Requirement: Session survives client disconnect
Closing the WebSocket (browser tab closed, refresh, network drop) SHALL kill only the attach pty — never the tmux session. A subsequent connection to the same id SHALL reattach to the live session with tmux's current screen content (including scrollback state held by tmux).

#### Scenario: Kill tab, reopen, history intact — the P0 gate
- **WHEN** the user closes the browser tab mid-Claude-response, then reopens the UI and reconnects to the same session
- **THEN** the tmux session was alive the whole time (verifiable via `tmux ls` during the gap)
- **THEN** the reattached terminal shows the session's current screen, including output produced while no client was attached

#### Scenario: Daemon restart does not kill sessions
- **WHEN** the daemon process is killed and restarted while a garage session exists
- **THEN** `GET /api/sessions` after restart lists the session and `WS /term/:id` reattaches successfully

#### Scenario: No orphaned ptys
- **WHEN** a WebSocket client disconnects
- **THEN** the daemon's corresponding pty process exits (no accumulating `tmux attach` processes after repeated connect/disconnect cycles)

### Requirement: Plain-terminal escape hatch preserved
Garage sessions SHALL remain reachable from any ordinary terminal via `tmux attach -t garage/<workspace>/<label>`, concurrently with a browser attach.

#### Scenario: Simultaneous iTerm and browser attach
- **WHEN** a session is attached in the browser and a user runs `tmux attach -t garage/garage-dev/main` in iTerm
- **THEN** both views mirror the same live session — typing in either appears in both
