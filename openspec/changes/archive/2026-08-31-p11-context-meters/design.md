# Design: p11-context-meters

## Context

Designed in the p8 mockup; data sources verified live 2026-08-30 (statusline stdin JSON carries `context_window.used_percentage` + `rate_limits`; transcript JSONL carries per-message usage; `claude agents --json` has nothing). Daemon-side per the two-UIs discipline. Sonnet implements, Fable reviews.

## Goals / Non-Goals

**Goals:** per-tile context %, account usage chip, chaining wrapper install with hook-install safety, transcript fallback so meters work with zero setup.
**Non-Goals:** cost display; per-message token history; web UI rendering (fields ready for it); pushing per-percent SSE events.

## Decisions

- **Ingest module mirrors hooks.js**: token auth, same session resolution (session_id → `claude agents --json` pid → pane pid; cwd fallback), in-memory store keyed by session id + one account-wide rate-limits slot. New `daemon/src/statusline.js`.
- **Wrapper**: a small shell command stored in settings.json's `statusLine` (merge logic lifted from hooks.js's parse/backup/atomic pattern): reads stdin once, POSTs to `http://127.0.0.1:4747/api/statusline/claude?token=…` via curl with `--max-time 1` fire-and-forget, then execs the user's previous command with the same stdin (captured to a temp var) or exits silently if none. Chaining metadata (the wrapped original) stored alongside so reinstall is idempotent and uninstall is possible later.
- **Transcript reader**: `daemon/src/transcript.js` — slug the session cwd the way Claude Code does (path with `/`→`-`), tail-read ≤256KB, scan backwards for the last assistant `message.usage`, window from a small model→window map (default 200k). Cached `{value, at}` per session; refreshed on demand from the sessions route when stale (>15s) and no statusline data, via a non-blocking queue (never delays the response — serve stale/null, refresh in background).
- **API shape locked in the proposal** so daemon and wall waves run in parallel.
- **Wall**: `context` on the session model; meter renderer in theme/tile (segments = round(pct/25) filled, min 1 when >0); red at ≥80 via a dedicated `ctx_hot` color (the palette's red, same as deletions — distinct from amber); strip chip fed by a 60s `GET /api/usage` poll in the runtime; install action wired as an overlay/help item calling the daemon.
- **e2e**: statusline POST with a fake payload (hookToken from scratch state) drives meter + chip assertions; fallback path asserted with a crafted transcript file under a scratch `~/.claude`-shaped dir? — no: transcript path derives from the real `~/.claude/projects`; e2e SHALL instead point the reader at a scratch root via env override `GARAGE_CLAUDE_HOME` (reader-only, default `~/.claude`) — small, testability-driven, documented.

## Risks / Trade-offs

- [Wrapper misbehavior corrupts the user's statusline] → chain-exec preserves output byte-for-byte; failure mode is "garage misses a post", never "statusline breaks"; e2e asserts the chained original's output.
- [Transcript reads on big files] → tail-bounded + 15s cache + background refresh; worst case meters lag, never a hot loop.
- [Model window map rots] → default 200k + map only for known exceptions; percentage is a hint, not billing.

## Open Questions

- None blocking.
