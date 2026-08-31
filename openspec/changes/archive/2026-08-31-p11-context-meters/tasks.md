# Tasks: p11-context-meters

## 1. Daemon telemetry

- [x] 1.1 `statusline.js`: ingest route (token, resolution, clamped context %, account rate limits), in-memory store; tests
- [x] 1.2 Wrapper install/snippet routes with chain-exec, hook-install safety (parse-or-refuse, backup, atomic, idempotent); tests
- [x] 1.3 `transcript.js`: slug/tail-read/usage-scan/window map + 15s cache + background refresh + `GARAGE_CLAUDE_HOME` override; tests
- [x] 1.4 `context` on GET /api/sessions (statusline > transcript > null; restorable null) and `GET /api/usage`; tests

## 2. Wall meters

- [x] 2.1 `context` on the session model; tile-bar meter (segments, dim/red thresholds, ladder priority above subtitle); unit tests incl. null-unchanged
- [x] 2.2 Strip usage chip with 60s /api/usage poll; hidden when null; tests
- [x] 2.3 Install affordance: one-time dim hint + install action calling the daemon, success/failure notices; help entry; tests

## 3. Verification

- [x] 3.0 Launcher staleness guard: when running from a source checkout, rebuild the dist binary if any wall/src file is newer than it (announced, cargo present; mirrors the stale-daemon gate) — root cause of the invisible-picker report

- [x] 3.1 e2e run_p11.sh: fake statusline POST → meter (dim and red cases) + chip; transcript fallback via GARAGE_CLAUDE_HOME fixture; chained-wrapper output byte-parity check against a fake pre-existing statusline; existing suites stay green; verification.md
