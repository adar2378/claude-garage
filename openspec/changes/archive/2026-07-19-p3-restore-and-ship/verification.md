# P3 e2e verification — 2026-07-19

## 3.1 Reboot simulation (the headline)

`tmux kill-server` with 4 live sessions (the real reboot scenario):
- All 4 immediately reported `status:"restorable"` by the API; rail showed dimmed ⟳ rows with per-session restore + per-workspace "restore all"; grid rendered placeholder cells instead of doomed terminals ✅
- "restore all" clicked in the UI for both workspaces → main, alpha, beta recreated via `claude --resume` with **conversation continuity confirmed** (alpha's prior paragraph exchange and beta's prior reply present in the resumed transcripts via `capture-pane`) ✅

**Bug found & fixed during verification:** a session that never had a submitted exchange (`side`) has no persisted conversation — `claude --resume` prints "No conversation found" and exits, killing the new tmux session and looping it back to restorable. First fix attempt raced (the exit takes >1.2s); final fix: two-stage liveness confirmation (1.5s + 2s) with fallback to a fresh `claude` session, targets restored concurrently so restore-all doesn't serialize the waits. Verified: `resumed:false` returned, fresh session alive and stable at +9s ✅

## 3.2 Fresh-install gate (the IDEA.md P3 gate)

`npm pack` → tarball installed into a pristine scratch dir → `./node_modules/.bin/claude-garage` (GARAGE_PORT=4801):
- **Bug found & fixed:** first run crashed — daemon deps (fastify, node-pty, ws, @fastify/static) were declared in the `daemon` workspace, which npm ignores for consumers of the packed root. Fixed by hoisting the runtime deps to the root package (dev workflow unaffected). This is exactly what the fresh-install gate exists to catch.
- After fix: API up, UI served by the daemon itself (no Vite), 4 sessions listed, **2 live terminals streaming over same-origin WebSockets**, ⟉ per-workspace controls all present ✅
- Tarball contents verified: bin, daemon/src, ui/dist only; `postinstall` chmod for node-pty's spawn-helper ships with the package ✅

## 3.3 Prereqs + shutdown

- PATH containing node+claude but no tmux → `tmux not found — install: brew install tmux`, exit 1, daemon never started ✅
- SIGTERM to the packaged process → daemon down, **all 4 tmux sessions alive** (the product promise, honored by the packaged binary) ✅

## 3.4 New keybindings

- `?` help overlay opens listing all 11 bindings (including `Ctrl + \``), Esc closes ✅
- `Ctrl+\`` with DOM focus inside a terminal → focus returned to chrome (BODY), keyboard-only blur works (P1's open question closed) ✅

## Verdict

**P3 gate: PASS — and with it, the full IDEA.md spec (P0–P3) is built and verified.** Two real bugs were caught by the e2e gates themselves (packaging deps, unresumable-session loop), which is the strongest argument for the per-phase verification discipline this project ran on.
