# tui-wall

## ADDED Requirements

### Requirement: Full-screen TUI client over the daemon API
`claude-garage tui` SHALL run a full-screen nocterm application that reads all workspace/session state exclusively from the daemon HTTP API (`GET /api/sessions`, `GET /api/workspaces`) and subscribes to `GET /api/events` (SSE) for changes. The TUI SHALL NOT use the `/term` WebSocket bridge and SHALL NOT parse tmux state itself.

#### Scenario: TUI reflects daemon state
- **WHEN** the TUI starts while the daemon reports two workspaces with three live sessions
- **THEN** the rail lists both workspaces with their sessions, and the grid shows tiles for the focused workspace's sessions, all sourced from `/api/sessions`

#### Scenario: SSE change triggers refetch
- **WHEN** an SSE `sessions` event arrives
- **THEN** the TUI refetches `/api/sessions` and reconciles the rail and grid without a restart

### Requirement: Live tiles attach through tmux
Each visible tile SHALL be an embedded terminal fed by a PTY running `tmux attach -t =<session-id>`, sized to the tile, with the `TMUX` environment variable removed from the child environment. Killing the TUI SHALL never terminate a tmux session.

#### Scenario: tmux stays the owner
- **WHEN** the TUI process is killed while five tiles are attached
- **THEN** all five tmux sessions remain alive and `tmux attach -t garage/<ws>/<label>` from a plain terminal still works

#### Scenario: Nested tmux does not refuse
- **WHEN** the TUI itself runs inside a tmux session
- **THEN** tile PTYs still attach successfully (no "sessions should be nested" refusal)

### Requirement: Tile PTY death recovery
When a tile's attach PTY exits while the daemon still lists the session as live, the TUI SHALL automatically reattach within 2 seconds. When the daemon reports the session gone or restorable, the tile SHALL show a placeholder naming the state instead of a dead terminal.

#### Scenario: Inner session recreated
- **WHEN** a tmux session is killed and recreated with the same name outside the TUI
- **THEN** the tile reattaches to the new session within 2 seconds without restarting the TUI

#### Scenario: Restorable placeholder
- **WHEN** the daemon lists a session with status `restorable`
- **THEN** its tile shows a restore placeholder (no PTY spawn is attempted)

### Requirement: PTY size tracks the tile
Each tile's attach PTY SHALL be resized to the tile's inner cell size after the attach starts and again on every tile rect change (grid reshape when tiles are added/removed, maximize/unmaximize, terminal resize). A resize that occurs before the PTY process is running SHALL be re-applied on its transition to running (the attach is deferred/staggered), and after any reattach. The tmux client behind a tile SHALL therefore never stay at the spawn default 80×24 once the tile has laid out.

#### Scenario: Attach client matches the tile
- **WHEN** a tile has rendered at an inner size of, e.g., 84×24 cells
- **THEN** `tmux list-clients` for that session reports `client_width`/`client_height` equal to the tile's inner size (not 80×24), and session content no longer double-wraps inside the tile

#### Scenario: Maximize resizes the PTY
- **WHEN** the focused tile is maximized to the full grid area
- **THEN** its PTY is resized to the maximized inner size, and back to the grid-cell size on unmaximize

### Requirement: Maximized tile
Pressing `m` in the garage layer SHALL toggle the focused tile between its grid cell and the full grid area (rail and strip stay visible). Esc SHALL NOT be a maximize control (Esc is never a garage binding). The maximized state SHALL live in the wall state (`maximizedSessionId`); moving focus to a different session SHALL exit maximize, and reconciliation SHALL clear it when the session leaves the grid. Sibling tiles SHALL stay mounted underneath (their terminal buffers survive the round-trip) but SHALL NOT receive wheel or click input while covered.

#### Scenario: m toggles full-grid
- **WHEN** the user presses `m` with a tile focused, then `m` again
- **THEN** the tile first takes the full grid area (rail/strip unchanged) and then returns to its grid cell, with its PTY resized both times

#### Scenario: Focus change exits maximize
- **WHEN** a tile is maximized and the user focuses another session (cycle keys, rail click, or the maximized session dying)
- **THEN** the grid returns to its normal layout

### Requirement: Empty states
A wall with zero workspaces SHALL render a centered onboarding panel in the grid area ("no workspaces yet — press w to add one; every Claude session in it appears here live") instead of a blank wall. A focused workspace with zero sessions SHALL render a tile-area hint naming the spawn keys ("press n for a session, N for a worktree session"). Empty-state text SHALL never use amber (amber stays exclusive to needs-input).

#### Scenario: Zero workspaces onboard
- **WHEN** the TUI starts with no registered workspaces and no sessions
- **THEN** the grid area shows the onboarding panel pointing at `w`, in neutral colors

#### Scenario: Empty workspace hints at spawn
- **WHEN** the focused workspace has no sessions
- **THEN** the tile area shows the `n`/`N` spawn hint instead of a blank grid

### Requirement: Grid layout with a six-tile cap
The grid SHALL show at most 6 live tiles for the focused workspace in a balanced grid (`ceil(sqrt(n))` columns, row-major). Sessions beyond the cap SHALL remain listed in the rail and reachable by focus keys; focusing an overflow session SHALL swap it into the grid.

#### Scenario: Seventh session overflows to rail
- **WHEN** a workspace has 7 live sessions
- **THEN** 6 tiles render and the 7th session appears in the rail only, marked as not gridded

#### Scenario: Focusing an overflow session swaps it in
- **WHEN** the user focuses the non-gridded session from the rail
- **THEN** it takes a grid slot (the least-recently-focused tile leaves the grid) and its terminal is live

### Requirement: Rail, strip, and salience ladder
The TUI SHALL render a workspace rail (workspaces with their sessions, glyphs `● ◐ ✓ ○ ⟳`) and a single-line bottom strip (workspace tabs with per-workspace needs-input dots, keys-target chip). Amber SHALL be used exclusively for `needs-input`: amber tile border and title, amber rail row, amber strip dot. `done` SHALL render green and stop being highlighted 2 minutes after the transition.

#### Scenario: Only blocked sessions are amber
- **WHEN** sessions in states needs-input, working, done, and idle are all visible
- **THEN** exactly the needs-input session renders amber (border, rail row); working/idle render in neutral tones; done renders green

#### Scenario: Done fades
- **WHEN** a session transitioned to `done` more than 2 minutes ago
- **THEN** its done highlight is no longer emphasized (glyph remains)

### Requirement: Performance under load
The TUI SHALL cap rendering at 15 frames per second and SHALL remain responsive under output load: with 6 tiles receiving a combined 200+ lines/second, a keypress SHALL be processed within 50 ms. The TUI SHALL disable terminal flow control (`IXON`) at startup so Ctrl+Q/Ctrl+S reach the application, and SHALL restore terminal settings on exit.

#### Scenario: Input latency under load
- **WHEN** 5 tiles stream heavy colored output and the user presses a workspace digit
- **THEN** the focus change renders within 50 ms of the keypress reaching the process

#### Scenario: Ctrl+Q reaches the app
- **WHEN** the user presses Ctrl+Q after startup
- **THEN** the application receives the key event (the tty driver does not consume it)
