## MODIFIED Requirements

### Requirement: macOS notification on needs-input while UI hidden
When a session transitions to `needs-input` while no garage client is visible (no TUI has sent a visibility heartbeat within its TTL), the daemon SHALL trigger a macOS notification identifying the session. The notification SHALL NOT open a URL when clicked. The daemon SHALL trigger at most one notification per needs-input transition — it SHALL NOT repeat the notification for as long as the session remains in `needs-input` without a further transition away and back.

#### Scenario: Notification fires when no TUI is visible
- **WHEN** no TUI is running and a session transitions from `working` to `needs-input`
- **THEN** a macOS notification is triggered identifying that session, with no click-to-open URL

#### Scenario: Notification does not repeat while status is unchanged
- **WHEN** a session has already triggered a needs-input notification and remains in `needs-input` with no further status transition
- **THEN** no additional notification is triggered for that same session while it stays in `needs-input`

#### Scenario: Renewed transition fires a new notification
- **WHEN** a session that previously notified for `needs-input` transitions to `working` and then back to `needs-input`
- **THEN** a new notification is triggered for this second `needs-input` transition
