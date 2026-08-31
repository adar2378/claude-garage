# session-status (delta)

## ADDED Requirements

### Requirement: Pane title exposure
The daemon SHALL capture each live garage session's tmux pane title (`#{pane_title}`) during its existing pane listing and expose it as `title` (string or null) on `GET /api/sessions` entries. A title equal to tmux defaults (empty, the hostname, or the bare shell/process name) SHALL be exposed as null rather than noise. Restorable entries have `title: null`. The value refreshes at poller cadence; no new tmux invocations are added (extend the existing `list-panes` format string).

#### Scenario: Claude Code title surfaces
- **WHEN** Claude Code in a session sets its OSC title to "✳ Refactoring the poller"
- **THEN** that session's listing entry includes `"title": "✳ Refactoring the poller"` within one poll interval

#### Scenario: Default titles are null
- **WHEN** a session's pane title is the machine hostname
- **THEN** the entry's `title` is null
