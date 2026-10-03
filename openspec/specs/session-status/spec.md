# session-status

## Purpose

Per-session state (needs-input / working / done / idle) fed by a hybrid of Claude Code HTTP hooks (precision) and a `claude agents --json` poller (zero-setup baseline), pushed to the UI over SSE, with macOS notifications for off-screen attention routing.
## Requirements
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

#### Scenario: Hook posts require the per-install token
- **WHEN** `POST /api/hooks/claude` arrives without the per-install token (generated once, embedded in the snippet URL, persisted in `~/.garage/state.json`) or with a wrong token
- **THEN** the daemon responds 401 and no session status changes

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

### Requirement: Idle reminders are not needs-input
Claude Code's Notification hook fires both for genuine blockers (permission prompts, questions, plan approvals) and for an idle reminder emitted after the prompt sits unused (~60s, message "Claude is waiting for your input"). The daemon SHALL ignore idle-reminder notifications — leaving the session's state untouched — and SHALL map every other Notification (including ones with no message) to `needs-input`, failing toward attention rather than away from it. This preserves the state vocabulary: `needs-input` means waiting for you to answer; a finished turn waiting for your next prompt is `done`/`idle`.

#### Scenario: Idle reminder leaves state alone
- **WHEN** a session's turn ended (Stop → `done`) and Claude Code later fires the idle-reminder Notification
- **THEN** the session's state is unchanged (`done`, decaying to `idle`), not `needs-input`

#### Scenario: Permission prompts still flag instantly
- **WHEN** a Notification arrives with a permission-request message (or no message at all)
- **THEN** the session transitions to `needs-input`

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

### Requirement: Pane title exposure
The daemon SHALL capture each live garage session's tmux pane title (`#{pane_title}`) during its existing pane listing and expose it as `title` (string or null) on `GET /api/sessions` entries. A title equal to tmux defaults (empty, the hostname, or the bare shell/process name) SHALL be exposed as null rather than noise. Restorable entries have `title: null`. The value refreshes at poller cadence; no new tmux invocations are added (extend the existing `list-panes` format string).

#### Scenario: Claude Code title surfaces
- **WHEN** Claude Code in a session sets its OSC title to "✳ Refactoring the poller"
- **THEN** that session's listing entry includes `"title": "✳ Refactoring the poller"` within one poll interval

#### Scenario: Default titles are null
- **WHEN** a session's pane title is the machine hostname
- **THEN** the entry's `title` is null

