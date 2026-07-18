# Tasks: p1-pit-wall

Every task ends with a binary gate. Groups 1–2 (daemon) and group 3 (UI) are parallelizable; group 4 integrates and verifies e2e.

## 1. Workspace registry + spawn change (daemon)

- [x] 1.1 Registry module: `~/.garage/state.json` `{workspaces:{name:{dir}}}`, lazy creation, atomic temp+rename writes
- [x] 1.2 `PUT /api/workspaces` upsert (`NAME_RE` validation, dir must exist → 400) and `GET /api/workspaces`
- [x] 1.3 `POST /api/sessions` breaking change: body `{workspace, label}`; dir resolved from registry; unknown workspace → 404; registered dir no longer exists → 400
- [x] 1.4 Gate: via curl — register workspace, spawn with `{workspace,label}` only, session lands in the registered dir (`tmux ls` + session_path); unknown workspace 404s; after `rmdir` of a registered dir spawn 400s; registry file survives daemon restart

## 2. Status engine (daemon)

- [x] 2.1 `StatusStore`: four states, `setStatus` single write path, edge-triggered pub-sub, `done`→`idle` decay (2 min), `needs-input` never auto-decays
- [x] 2.2 Poller (2s): `claude agents --json` joined to garage sessions via `tmux list-panes -a` pane_pid == agents pid; `busy`→`working`, `idle`→`idle`; never downgrades `needs-input`; cwd fail-open fallback when the pid join misses
- [x] 2.3 Hook receiver `POST /api/hooks/claude`: `Notification` → `needs-input`, `Stop` → `done`; session resolution via payload session_id→agents-json→pane_pid join, cwd fail-open fallback; plus `GET /api/hooks/snippet` returning the exact settings.json block (HTTP hooks + `allowedHttpHookUrls`)
- [x] 2.4 `GET /api/events` SSE stream emitting status transitions; `status` field added to `GET /api/sessions`; UI-visibility tracking endpoint so the daemon knows when no foreground page is watching
- [x] 2.5 macOS notification via `osascript` on transition INTO `needs-input` while no foreground page; edge-triggered, once per transition; darwin-only no-op elsewhere
- [x] 2.6 Gate (verification spike): with a real claude session — hook POST flips store and SSE event arrives < 2s; with hooks absent, poller alone reports working/idle correctly; pid join maps two same-dir sessions to distinct ids

## 3. Pit wall UI

- [x] 3.1 Workspace rail: registered workspaces with nested sessions + status glyphs (● ◐ ✓ ○), needs-you-first ordering (blocked workspaces first, blocked sessions first within group), live updates from SSE without reload
- [x] 3.2 Terminal grid: ALL sessions of the focused workspace as concurrent live xterm terminals (reuse SessionTerminal), stacked; exactly one focused with highlight; click to focus; switching workspace swaps grid connections (old ptys closed)
- [x] 3.3 Keybindings: `1–9` workspace switch, `[`/`]` cycle terminal, `a` cross-workspace jump to a needs-input session; suppressed while a terminal has DOM focus; click outside terminals blurs back to chrome-navigation mode
- [x] 3.4 New-session control per workspace (label prompt → spawn API) + add-workspace control (name+dir → PUT /api/workspaces) + one-time banner when status is poller-only (hooks not installed), linking the snippet endpoint
- [x] 3.5 Gate: with two workspaces × two sessions each — both terminals of the focused workspace visibly streaming at once, zero window switches; keybindings work and are suppressed while typing into a terminal

## 4. Integration + e2e verification (the P1 phase gate)

- [x] 4.1 Hook snippet installed into `~/.claude/settings.json` (with user-file backup); real permission prompt in a real session flips the rail glyph to ● and fires SSE < 2s
- [x] 4.2 Full pit-wall scenario (Playwright): two workspaces, four sessions; grid streams two terminals simultaneously; `1-9`/`[`/`]`/`a` verified including `a` jumping across workspaces; typed digits go to the pty, not the chrome
- [x] 4.3 macOS notification observed (or its osascript invocation captured) when needs-input fires with the page hidden; no repeat while blocked
- [x] 4.4 Needs-you-first ordering verified live (blocked workspace bubbles up without reload)
- [x] 4.5 Record everything in `verification.md`; only then is P1 done
