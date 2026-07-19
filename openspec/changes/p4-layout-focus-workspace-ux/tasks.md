# Tasks: p4-layout-focus-workspace-ux

Groups 1 (daemon), 2 (layout UI), 3 (chrome UI) are parallelizable; group 4 wires and verifies e2e.

## 1. Daemon endpoints

- [ ] 1.1 `POST /api/pick-directory`: darwin `osascript choose folder` → `{dir}` (trailing slash stripped) / `{cancelled:true}` / 501 elsewhere; long timeout while dialog is open
- [ ] 1.2 `PATCH /api/workspaces/:name` `{name}`: 400/404/409 guards; renames registry key, live tmux sessions (`rename-session`, attached clients survive), and resume-metadata keys in one pass; partial tmux failures reported not rolled back
- [ ] 1.3 Gate: rename flow e2e via curl with a live test session — tmux name, GET /api/sessions, and state.json all show the new name; guards verified

## 2. Layout UI (dockview + pop-out)

- [ ] 2.1 `lib/popouts.js` heartbeat protocol (5s beat, 15s TTL, storage events); `SoloView` full-window terminal for `/?solo=<id>`
- [ ] 2.2 Grid → dockview: panel per session, drag-dock with splitters, per-workspace layout persistence + reconcile (join/leave/reset), focused-panel ↔ app focus sync, pop-out button per cell, popped-out placeholder with reclaim
- [ ] 2.3 Gate: `npm run build -w ui` clean; layouts survive reload; killing a session removes its panel

## 3. Chrome UI (settings, dimming, picker, nesting, rename)

- [ ] 3.1 `lib/settings.js` + gear `SettingsPopover` (focus-dim toggle, localStorage, live across windows)
- [ ] 3.2 Focus-dim zones + CSS: rail rows and cells dim at 0.45 except `needs-input` (never) and the focused surface; resolution order help → review → DOM-focused cell → app-focused cell
- [ ] 3.3 Picker-first add-workspace flow (auto-name, editable, collision suffix, manual fallback); inline workspace rename; nested rail rendering from path containment with flattened 1–9 order
- [ ] 3.4 Gate: build clean; WIRING.md delivered for App.jsx integration

## 4. Integration + e2e verification (the P4 gate)

- [ ] 4.1 App.jsx wiring applied (settings hook, focus-dim class + zone tags on grid cells, gear button, flattened-order keybindings)
- [ ] 4.2 Drag-dock e2e (Playwright): drag one sandbox cell beside the other → side-by-side layout; survives reload; reset-layout restores stack
- [ ] 4.3 Pop-out e2e: pop a session out → solo window live, main grid shows placeholder; close window → cell reclaims within TTL
- [ ] 4.4 Focus-dim e2e: toggle on via gear → unfocused zones dim, focused cell bright; force a session to needs-input → its row/cell stay full brightness while dimming is on; toggle off → instant restore
- [ ] 4.5 Picker e2e (real dialog on screen): add workspace via native picker → auto-named, editable, appears in rail; nested case: add a subdirectory of an existing workspace → renders indented under it; rename a workspace inline → tmux sessions + rail + state.json all follow
- [ ] 4.6 Record in `verification.md`
