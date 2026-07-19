# connection-resilience

## Purpose

Terminal WebSocket auto-reconnect, per-cell reconnect overlays, and the header connection-state chip — stale state is never silently presented as fresh.

## Requirements

### Requirement: Terminal WebSocket auto-reconnect
When a session terminal's `WS /term/:id` connection closes while the session is still expected to be live, the client SHALL attempt to reconnect automatically with backoff (rather than printing a terminal `[detached]` end state that persists until page reload). On successful reconnect the terminal SHALL resume rendering the live pty stream without a page reload.

#### Scenario: Daemon restart recovers without a page reload
- **WHEN** the daemon process restarts while a workspace's terminals are on screen, then comes back up
- **THEN** each terminal reconnects automatically and resumes streaming its tmux pane, with no page reload required

#### Scenario: Reconnect stops for a session that no longer exists
- **WHEN** a terminal's WS closes because the underlying session was killed (it no longer appears in `GET /api/sessions`)
- **THEN** the client stops retrying for that id rather than reconnecting forever

### Requirement: Per-cell reconnect overlay
While a terminal's connection is down, its cell SHALL display an overlay stating that the connection to the daemon was lost and that the tmux session itself is still alive, with a manual reconnect control. The overlay SHALL clear as soon as the connection is re-established (automatically or via the control).

#### Scenario: Overlay appears on drop and clears on recovery
- **WHEN** a terminal's WS drops (e.g. after machine sleep) and later reconnects
- **THEN** the cell shows the disconnected overlay with a reconnect control while down, and returns to the live terminal view once reconnected

### Requirement: Connection-state chip
The app header SHALL display a connection-state indicator reflecting daemon reachability: a "live" state when the SSE event stream is connected, and a "reconnecting" state while it is down or retrying. When the SSE stream reconnects after a drop, the client SHALL resync sessions (existing behavior) and the chip SHALL return to "live", so stale status is never silently presented as fresh.

#### Scenario: SSE drop is visible in the header
- **WHEN** the daemon becomes unreachable and the SSE connection errors
- **THEN** the header chip changes from "live" to "reconnecting" while the browser retries

#### Scenario: Recovery resyncs and shows live
- **WHEN** the SSE connection re-establishes after a drop
- **THEN** the client refetches sessions and the chip shows "live" again
