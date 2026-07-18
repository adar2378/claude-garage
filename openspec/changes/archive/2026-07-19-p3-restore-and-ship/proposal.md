# Proposal: p3-restore-and-ship

## Why

The garage now does everything the thesis promised — but only until the Mac reboots (tmux dies, sessions vanish, conversations orphaned) and only for someone willing to run two dev servers from a checkout. P3 makes it survive reboots (`claude --resume` brings conversations back) and installable (`npx claude-garage`), which is the difference between a prototype and the OSS tool the competitive timing argues for.

## What Changes

- **Session metadata for restore**: the daemon records, per garage session, the Claude Code `sessionId` (already observed by the poller via `claude agents --json`) plus workspace/label into `~/.garage/state.json` — updated opportunistically, removed when a session is deliberately killed via the API (not when tmux dies out from under it).
- **Reboot restore**: on `GET /api/sessions`, sessions present in state.json but absent from tmux are reported as `restorable`. `POST /api/sessions/restore` recreates one (or all) via `tmux new-session … claude --resume <sessionId>` in the workspace dir. UI: restorable sessions render dimmed in the rail with a restore control, plus a "restore all" affordance when tmux came back empty.
- **Keyboard-only terminal blur** (the P1 open question): a dedicated chord (`Ctrl+\``) blurs the focused terminal back to chrome-navigation mode — chosen because tmux/readline/Claude Code leave it unbound. Documented in a small `?` help overlay listing all keybindings.
- **npx packaging**: `package.json` gains `bin: {"claude-garage": "bin/garage.js"}`; the bin builds nothing at runtime — the published package ships the pre-built UI (`ui/dist`), the daemon serves it as static files (same origin, so the Origin allowlist gains the daemon's own origin), starts on 127.0.0.1:4747, and prints/opens `http://127.0.0.1:4747`. `prepack` runs the UI build. Dev workflow (Vite + proxy) unchanged.
- **README**: install (`npx claude-garage`), prerequisites (tmux, Claude Code CLI, macOS), the hook snippet step, keybindings table, escape-hatch philosophy. SEO line per IDEA.md ("Claude Code session manager — parallel sessions, grouped terminals, diff review").

Out of scope: publishing to npm (user's call, needs their npm account), Linux notification support, tmux-native deck layout, auto-update.

## Capabilities

### New Capabilities
- `session-restore`: restorable-session detection, resume metadata lifecycle in state.json, the restore endpoint, and conversation continuity via `claude --resume`.
- `packaging`: the `npx claude-garage` entrypoint contract — single process serving UI + API on loopback, prerequisites check with actionable errors (tmux missing, claude missing), graceful shutdown that never kills sessions.

### Modified Capabilities
- `pit-wall-ui`: adds the keyboard blur chord and the `?` help overlay to the keybinding requirements; restorable sessions appear in the rail.

## Impact

- Daemon: state.json schema grows a `sessions` map; static file serving (`@fastify/static` — first new daemon dependency, justified: hand-rolling static serving with correct MIME/caching is worse); bin entrypoint with prereq checks.
- UI: restore affordances, help overlay, blur chord; build output ships in the package.
- Security: serving UI from 4747 means adding `http://127.0.0.1:4747` to the default Origin allowlist.
- Packaging gate needs a clean-machine simulation: `npm pack` → install the tarball in a scratch dir → `npx claude-garage` → full flow works.
