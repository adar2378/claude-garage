# Proposal: p11-context-meters

## Why

Context pressure is a planning signal a pit wall should surface: an agent at 88% context needs a compact or restart soon, and account rate limits (`/usage`) decide whether to start more work. Claude Code exposes both — richly via the statusline stdin JSON (`context_window.used_percentage`, `rate_limits.five_hour/seven_day`), and as a fallback via each session's transcript JSONL. Verified on this machine (2026-08-30): `claude agents --json` has no token data; statusline + transcript are the sources.

## What Changes

- **Daemon ingest (primary)**: a `POST /api/statusline/claude?token=` endpoint (hook-token authed, same resolution as hooks: statusline `session_id` → garage session). Garage installs a *chaining* statusline wrapper into `~/.claude/settings.json` (same machinery/UX as hook install: parse-or-refuse, backup, idempotent, execs any pre-existing statusline so nothing is clobbered) that POSTs the JSON to the daemon.
- **Transcript fallback**: for sessions with no statusline data, the daemon derives context usage from the transcript JSONL's last assistant `usage` (input + cache_read + cache_creation vs the model's window), refreshed lazily (tail-read only, ≥15s per session, never blocking a poll tick).
- **API (shape locked here so UI work can parallelize)**: session entries gain `context: { usedPercentage: number, source: "statusline"|"transcript" } | null`; new `GET /api/usage` returns `{ fiveHour: {usedPercentage, resetsAt} | null, sevenDay: {...} | null }` (null until statusline data arrives; account-wide, from the most recent statusline post).
- **Wall UI**: tile-bar context meter (`▰▰▱▱ 42%` style) — quiet/dim below 80%, red at ≥80% (amber stays exclusive to needs-input); a strip usage chip (`5h 24% · wk 61%`) when `/api/usage` has data; statusline-install affordance surfaced like the hooks path (a dim one-time strip hint + the install action via the daemon).
- Web UI: untouched (fields are there for it later).

## Capabilities

### New Capabilities
- `context-telemetry`: daemon statusline ingest, wrapper install, transcript fallback, `context` field, `/api/usage`.
- `tui-context-meters`: tile meter, thresholds/colors, strip usage chip, install hint.

### Modified Capabilities
_None (session-status listing shape only gains an optional field; no existing requirement changes)._

## Impact

- Daemon: new route + wrapper-install module (mirrors hooks.js), lazy transcript reader, poller untouched except plumbing. Tests mirror the hooks/status suites.
- Wall: state field + two renderers + install action. e2e: meter/threshold/chip/install assertions.
- User's `~/.claude/settings.json` is modified only via the explicit install action (backup kept), exactly like hooks.
