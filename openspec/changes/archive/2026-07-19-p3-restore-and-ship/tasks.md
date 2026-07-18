# Tasks: p3-restore-and-ship

Groups 1 (daemon + packaging) and 2 (UI + README) are parallelizable; group 3 integrates and verifies e2e.

## 1. Restore + packaging (daemon)

- [x] 1.1 state.json `sessions` map: poller writes `{claudeSessionId, workspace, label}` per garage session on change only; `DELETE /api/sessions/:id` removes the entry; vanishing without API kill keeps it
- [x] 1.2 `GET /api/sessions` merges restorable orphans (status `restorable`); `POST /api/sessions/restore` `{id}` / `{all:true}` → `tmux new-session … claude --resume <sessionId>`; 409 + metadata kept on missing workspace/dir or name collision
- [x] 1.3 `bin/garage.js`: tmux/claude prereq checks (actionable errors, exit 1), starts daemon with `GARAGE_SERVE_UI=1`, prints + best-effort opens the URL; EADDRINUSE → readable `GARAGE_PORT` hint; SIGINT/SIGTERM close server without touching tmux
- [x] 1.4 Daemon serves `ui/dist` via `@fastify/static` (flag-gated); Origin allowlist gains `http://127.0.0.1:4747` + localhost variant; root `package.json`: `bin`, `files`, `prepack` (ui build)
- [x] 1.5 Gate: kill a test session's tmux out-of-band → GET shows it restorable → restore → conversation content present in the reattached terminal; deliberate DELETE leaves no restorable ghost

## 2. UI polish + README

- [x] 2.1 Restorable sessions in rail: dimmed row + `⟳` glyph + restore control; restore-all control when a workspace's sessions are all restorable; restored session appears live in the grid
- [x] 2.2 `Ctrl+\`` blur chord via xterm `attachCustomKeyEventHandler` (works while terminal focused); `?` help overlay listing every binding, Esc closes, suppressed in-terminal
- [x] 2.3 README.md: install (npx), prereqs, hook setup, keybindings table, escape-hatch philosophy, development; SEO description line
- [x] 2.4 Gate: `npm run build -w ui` clean; help overlay lists all bindings incl. the chord

## 3. Integration + e2e verification (the P3 phase gate)

- [x] 3.1 Reboot simulation: `tmux kill-server` with live sessions → rail shows all sessions restorable → restore-all → sessions back with conversations intact (`--resume` continuity verified by prior transcript content on screen)
- [x] 3.2 Fresh-install gate: `npm pack` → install tarball in scratch dir → `npx claude-garage` → UI served from 4747, terminals + diff + status all work in the packaged app (no Vite)
- [x] 3.3 Prereq + shutdown checks: PATH without tmux → actionable error; Ctrl+C on the packaged process → sessions survive, restart reattaches
- [x] 3.4 `Ctrl+\`` blurs a focused terminal (keyboard-only), `?` overlay opens/closes
- [x] 3.5 Record in `verification.md`; only then is P3 — and the whole spec — done
