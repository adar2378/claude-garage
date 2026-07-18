# P0 e2e verification — 2026-07-19

Environment: macOS (Darwin 25.5.0), Node v22.22.3, tmux 3.7b, Claude Code 2.1.214, node-pty 1.1.0. Daemon on `127.0.0.1:4747`, Vite on `127.0.0.1:5173`.

## Gate 0.1 — scaffold

| Check | Result |
|---|---|
| `npm run dev` starts daemon + UI (via concurrently) | ✅ |
| `GET /api/health` direct | ✅ `{"status":"ok"}` |
| `GET /api/health` through Vite proxy | ✅ `{"status":"ok"}` |
| Page loads (`<title>claude-garage</title>`) | ✅ |
| `lsof`: daemon bound to `127.0.0.1:4747` only | ✅ |

## Gate 0.2 — session lifecycle (curl vs real tmux)

| Check | Result |
|---|---|
| POST spawn → 201, appears in `tmux ls` and `GET /api/sessions` | ✅ |
| Duplicate spawn → 409 | ✅ |
| Invalid label (`Bad.Label`) → 400 | ✅ |
| DELETE non-garage session (`personal-scratch`) → 403, session untouched | ✅ |
| External `tmux kill-session` → vanishes from next GET, no restart | ✅ |
| DELETE garage session → 204, gone from tmux | ✅ |
| No tmux server → GET returns `[] [200]` | ✅ |

## Gate 0.3 — terminal bridge

- Protocol level (two concurrent WS clients): both received identical initial screen (6,881 bytes); typing via client A echoed to **both** A and B (+3,159 bytes each) — tmux mirroring works through the bridge; resize control frames accepted. ✅
- Browser (user-witnessed + Playwright): Claude Code TUI rendered in xterm.js at `127.0.0.1:5173`; prompt typed in browser, streamed response rendered live. ✅
- User exited `claude` inside the session → tmux session ended, `GET` returned `[]`, zero stray processes (design D7 confirmed by accident). ✅

## Gate 0.4 — survival (the P0 phase gate)

| Test | Result |
|---|---|
| **Tab death:** prompt sent ("count to 20 … SURVIVAL-TEST-DONE"), tab closed mid-response | ✅ `tmux ls` showed session alive throughout; Claude finished the response with **no client attached** (verified via `tmux capture-pane`) |
| **Reattach:** reopened browser | ✅ terminal showed the full output produced while detached |
| **Daemon death:** killed daemon+Vite, then restarted | ✅ session survived; `GET` listed it after restart; browser reattached with history |
| **Orphan check:** 10 WS connect/disconnect cycles | ✅ 0 stray `tmux attach` processes after all clients closed; session alive |

## Security hardening (post-gate)

Automated security review flagged CSRF + cross-site WebSocket hijacking against the loopback daemon (browser pages as confused deputies). Fixed with an Origin allowlist (`daemon/src/security.js`), verified:

| Check | Result |
|---|---|
| `POST /api/sessions` with `Origin: http://evil.example` | ✅ 403, nothing spawned |
| `DELETE` with foreign Origin | ✅ 403 |
| WS upgrade with foreign Origin | ✅ socket destroyed, no pty |
| WS upgrade with UI Origin (`http://127.0.0.1:5173`) | ✅ connects |
| curl / scripts with no Origin header | ✅ unaffected |
| `GET` (read-only) with any Origin | ✅ allowed |

## Verdict

**P0 gate: PASS.** tmux owns the sessions; the daemon and browser are genuinely disposable. Architecture thesis validated.

Notes for later phases:
- node-pty prebuilt `spawn-helper` loses its +x bit on `npm install` — handled by root `postinstall`; keep for packaging (P3).
- tmux "smallest client wins" sizing accepted for P0 (design risk); revisit in P1 grid.
- The tmux server inherits PATH from the daemon's environment; packaged startup (P3) must ensure `claude` is resolvable.
