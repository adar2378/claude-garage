## ADDED Requirements

### Requirement: Session status state model
Every garage session SHALL have exactly one status at any time, drawn from the set: `needs-input`, `working`, `done`, `idle`. A session with no status signal ever recorded SHALL default to `idle`.

#### Scenario: Newly spawned session starts idle or working
- **WHEN** a session is spawned and no status signal has yet been received for it
- **THEN** its status is `idle` (or `working`, if the spawn itself is treated as a status signal) — never `needs-input` or `done` without a corresponding signal

#### Scenario: Unknown session defaults to idle
- **WHEN** a session id has no status recorded for it (e.g. a session discovered via `tmux ls` for which no hook/poll signal has ever arrived)
- **THEN** its reported status is `idle`

### Requirement: Status included in session listing
`GET /api/sessions` entries SHALL include each session's current `status` field, using the same values as the state model.

#### Scenario: Status field present on every session
- **WHEN** `GET /api/sessions` is called while sessions `garage/kowboy/checkout` (needs-input) and `garage/kowboy/build` (working) exist
- **THEN** the response includes both sessions, each with a `status` field set to `needs-input` and `working` respectively

### Requirement: Status change push channel
The daemon SHALL expose `GET /api/events` as a Server-Sent Events stream that delivers a status-change event for a session whenever that session's status transitions from one value to another. A client connected to `/api/events` SHALL observe the event within 2 seconds of the underlying status-changing signal (hook delivery or poll detection) occurring.

#### Scenario: UI receives status change promptly
- **WHEN** a client is connected to `GET /api/events` and a session's status changes from `working` to `needs-input`
- **THEN** the client receives an event identifying the session and its new status `needs-input` within 2 seconds of the underlying signal

#### Scenario: No change, no event
- **WHEN** a client is connected to `GET /api/events` and no session's status changes
- **THEN** no status-change event is emitted for that period (aside from any protocol-level keepalive)

### Requirement: Status source is pluggable
The daemon SHALL determine session status from one or more status sources — a hook receiver endpoint and/or a poller — without the observable status API (`GET /api/sessions` status field, `/api/events` stream) depending on which source produced the signal. `POST /api/hooks/claude` SHALL accept Claude Code hook payloads and SHALL update the corresponding session's status based on the payload's event type and project directory.

#### Scenario: Hook payload updates status
- **WHEN** `POST /api/hooks/claude` receives a `Notification` hook payload for a project directory mapped to session `garage/kowboy/checkout`
- **THEN** that session's status becomes `needs-input`, observable via `GET /api/sessions` and via `/api/events`

#### Scenario: Stop hook marks a session done
- **WHEN** `POST /api/hooks/claude` receives a `Stop` hook payload for a project directory mapped to a session currently `working`
- **THEN** that session's status becomes `done`

#### Scenario: Observable behavior unchanged regardless of source
- **WHEN** a session's status becomes `needs-input` via a hook payload in one deployment, or via a poller detecting the equivalent condition in another deployment
- **THEN** in both cases `GET /api/sessions` reports `needs-input` for that session and `/api/events` emits an equivalent status-change event — no client-visible difference between the two mechanisms

### Requirement: macOS notification on needs-input while UI hidden
When a session transitions to `needs-input` while no pit-wall UI is visible (no connected client with an active/foreground page), the daemon SHALL trigger a macOS notification identifying the session. The daemon SHALL trigger at most one notification per needs-input transition — it SHALL NOT repeat the notification for as long as the session remains in `needs-input` without a further transition away and back.

#### Scenario: Notification fires when pit wall is not open
- **WHEN** no browser client is connected to the pit wall and a session transitions from `working` to `needs-input`
- **THEN** a macOS notification is triggered identifying that session

#### Scenario: Notification does not repeat while status is unchanged
- **WHEN** a session has already triggered a needs-input notification and remains in `needs-input` with no further status transition
- **THEN** no additional notification is triggered for that same session while it stays in `needs-input`

#### Scenario: Renewed transition fires a new notification
- **WHEN** a session that previously notified for `needs-input` transitions to `working` and then back to `needs-input`
- **THEN** a new notification is triggered for this second `needs-input` transition
