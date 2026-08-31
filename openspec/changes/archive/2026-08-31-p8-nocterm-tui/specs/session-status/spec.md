# session-status (delta)

## ADDED Requirements

### Requirement: Needs-input message capture
When a Notification hook event transitions a session to `needs-input`, the daemon SHALL record that event's message text for the session. The recorded message SHALL be cleared when the session leaves `needs-input`. Poller-sourced `needs-input` transitions (which carry no message) SHALL leave any recorded message unset rather than inventing one.

#### Scenario: Hook message recorded
- **WHEN** a Notification hook arrives with message "Claude needs your permission to use Bash" and the session transitions to needs-input
- **THEN** the daemon associates that message with the session

#### Scenario: Message cleared on answer
- **WHEN** the same session later transitions to `working`
- **THEN** the recorded message is cleared

### Requirement: Message exposed in session listing
`GET /api/sessions` entries with status `needs-input` SHALL include a `message` field carrying the recorded notification text, or `null` when none was captured. Entries in other states SHALL have `message` as `null`.

#### Scenario: Message present for blocked session
- **WHEN** `GET /api/sessions` is called while a hook-signalled needs-input session exists
- **THEN** that entry includes `"message": "<the notification text>"` and non-blocked entries include `"message": null`
