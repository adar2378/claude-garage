# context-telemetry Specification

## Purpose
TBD - created by archiving change p11-context-meters. Update Purpose after archive.
## Requirements
### Requirement: Statusline ingest
The daemon SHALL accept `POST /api/statusline/claude?token=<hookToken>` with Claude Code's statusline stdin JSON. It SHALL resolve the payload's `session_id` to a garage session (same strategy as hook resolution) and record: `context_window.used_percentage` (clamped 0–100) and, account-wide, `rate_limits.five_hour`/`seven_day` `{used_percentage, resets_at}` when present. Invalid token → 401; unresolvable session → `{ignored:true}` without error. Data is in-memory (like status), cleared when a session's record is dropped.

#### Scenario: Statusline post updates context
- **WHEN** a valid post carries `context_window.used_percentage: 42` for a resolvable session
- **THEN** that session's listing entry shows `context: {usedPercentage: 42, source: "statusline"}` on the next fetch

### Requirement: Chaining wrapper install
`POST /api/statusline/install` SHALL merge a garage statusline command into `~/.claude/settings.json` with the hook-install guarantees: parse-or-refuse (422 on corrupt JSON, file untouched), timestamped backup, atomic write, idempotent. If a statusLine command already exists and is not garage's, the installed wrapper SHALL chain it: forward stdin JSON to the daemon (fire-and-forget, short timeout, silent on failure) AND exec the original command with the same stdin so the user's statusline output is unchanged. With no pre-existing statusline, the wrapper posts and prints nothing. `GET /api/statusline/snippet` SHALL return the wrapper definition for manual installs.

#### Scenario: Existing statusline is chained, not clobbered
- **WHEN** the user has ccstatusline configured and installs garage's wrapper
- **THEN** their statusline output is byte-identical to before, and the daemon starts receiving posts

#### Scenario: Corrupt settings refused
- **WHEN** `~/.claude/settings.json` contains invalid JSON
- **THEN** install returns 422 and the file is untouched

### Requirement: Transcript fallback
For a live session with a known `claudeSessionId` and no statusline data, the daemon SHALL derive context usage from the session's transcript JSONL (`~/.claude/projects/<slug>/<id>.jsonl`): the last assistant message's `usage` (`input_tokens + cache_read_input_tokens + cache_creation_input_tokens`) over the model's context window (from the message's `model`; 200000 default when unknown). Reads SHALL be tail-bounded (last ≤256KB), at most once per 15s per session, off the poll hot path. Statusline data, when it later arrives, SHALL take precedence.

#### Scenario: Fallback without statusline
- **WHEN** no wrapper is installed and a session's transcript's last assistant usage sums to ~150k with a 200k-window model
- **THEN** its entry shows `context: {usedPercentage: 75, source: "transcript"}` within one refresh interval

### Requirement: Usage endpoint
`GET /api/usage` SHALL return `{fiveHour: {usedPercentage, resetsAt}|null, sevenDay: {usedPercentage, resetsAt}|null}` from the most recent statusline post (account-wide), nulls before any post arrives.

#### Scenario: Usage after a post
- **WHEN** a statusline post carried `rate_limits.five_hour.used_percentage: 23.5`
- **THEN** `GET /api/usage` returns `fiveHour.usedPercentage: 23.5`

### Requirement: Context on the session listing
Session entries SHALL include `context` (`{usedPercentage, source}` or null). Restorable entries: null. The SSE `sessions` push cadence is unchanged; context changes surface on fetches (no per-percent push).

#### Scenario: Field shape
- **WHEN** `GET /api/sessions` is called with one statusline-fed, one transcript-fed, and one cold session
- **THEN** entries carry `context` with sources "statusline", "transcript", and null respectively

