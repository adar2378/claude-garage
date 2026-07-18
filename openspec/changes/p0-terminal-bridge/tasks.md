# Tasks: p0-terminal-bridge

Every task ends with a binary gate — a "do X, see Y" check. A task is not done until its gate is demonstrated.

## 1. Scaffold (checkpoint 0.1)

- [ ] 1.1 Init npm-workspaces monorepo with `daemon/` and `ui/` packages; pin Node ≥ 20 in engines; commit lockfile
- [ ] 1.2 Daemon: Fastify app with `GET /api/health` → `{"status":"ok"}`, listening on `127.0.0.1` explicitly
- [ ] 1.3 UI: Vite + React app boots; Vite dev proxy forwards `/api` and `/term` to the daemon port
- [ ] 1.4 Gate 0.1: `npm run dev` starts both; `curl 127.0.0.1:<port>/api/health` returns ok; browser page loads via Vite and can fetch `/api/health` through the proxy; `lsof` confirms loopback-only bind

## 2. Session lifecycle (checkpoint 0.2)

- [ ] 2.1 Name validation: `workspace`/`label` must match `[a-z0-9-]+` (400 otherwise); id format `garage/<workspace>/<label>`
- [ ] 2.2 `POST /api/sessions` — `tmux has-session` collision check (409), then `tmux new-session -d -s <id> -c <dir> claude` (201)
- [ ] 2.3 `GET /api/sessions` — parse `tmux ls -F` filtered to `garage/` prefix; empty list (not error) when tmux server is down
- [ ] 2.4 `DELETE /api/sessions/:id` — `tmux kill-session`, 403 for any id outside `garage/` prefix
- [ ] 2.5 Gate 0.2: via curl — spawn shows up in `tmux ls` and in `GET /api/sessions`; duplicate spawn 409s; killing a session in iTerm makes it vanish from the next GET; DELETE of a non-garage name 403s and leaves it alive

## 3. Terminal bridge (checkpoint 0.3)

- [ ] 3.1 Daemon: `WS /term/:id` upgrade → spawn node-pty running `tmux attach -t <id>`; binary frames ⇄ pty bytes; JSON text frame `{type:"resize",cols,rows}` → `pty.resize()`
- [ ] 3.2 Pty lifetime owned by the socket: WS close kills the pty (detach, never kill-session); sweep guards against orphaned attach processes
- [ ] 3.3 UI: xterm.js + fit addon rendering the WS stream for one session (hardcoded id acceptable); keystrokes sent as binary; resize observer sends control frames
- [ ] 3.4 Gate 0.3: from the browser terminal, hold a real conversation with Claude Code — prompt typed, streamed response rendered; window resize reflows; simultaneous `tmux attach` in iTerm mirrors both ways

## 4. E2E survival verification (checkpoint 0.4 — the P0 phase gate)

- [ ] 4.1 Tab-death test: close the tab mid-response; verify via `tmux ls` the session lived; reopen and reattach — current screen restored including output produced while detached
- [ ] 4.2 Daemon-death test: kill and restart the daemon; session still listed and reattachable
- [ ] 4.3 Orphan check: 10 connect/disconnect cycles leave zero stray `tmux attach` processes (`ps` count)
- [ ] 4.4 Record the e2e run (steps + results) in the change folder as `verification.md`; only then is P0 done
