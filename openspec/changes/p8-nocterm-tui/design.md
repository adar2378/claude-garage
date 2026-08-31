# Design: p8-nocterm-tui

## Context

The daemon (Fastify, `daemon/src/`) already brokers everything a client needs: sessions, workspaces, status via SSE, hooks, diff, worktrees. The web UI is a thin client; the TUI is a second thin client. Two prior artifacts govern this change: the UX mockup (claude.ai artifact `7dc66760-7ad9-4728-b414-28dfe573af50` — Wall, key layers, triage queue, salience ladder) and the spike (`spikes/nocterm-wall/SPIKE.md` — working code for grid, encoder, patches, and all measured limits). The spike code is reference material; the product code is written fresh in `tui/`.

## Goals / Non-Goals

**Goals:**
- A daily-drivable terminal wall covering pillars 1–3 (tmux-owned terminals, live grid, needs-input triage) at the quality bar of the mockup.
- All spike learnings encoded as tests, not tribal knowledge.
- Any new logic lands daemon-side when both UIs could use it (this phase: the needs-input message).

**Non-Goals:**
- Review mode, review comments, overview (`0`), fleet restart, context meters, command palette, pit pet, worktree finish modal (later phases).
- Web UI changes of any kind.
- Linux/Windows support (macOS arm64 first; keep Dart code platform-clean).
- Upstreaming the nocterm patches (file issues/PRs opportunistically, never block on them).

## Decisions

- **Dart workspace layout**: `tui/` with `pubspec.yaml`, `bin/garage_tui.dart`, `lib/` (state, api client, encoder, widgets), `test/`. `tui/vendor/nocterm` is the vendored fork via `dependency_overrides` (start from the spike's `spikes/nocterm-wall/vendor/nocterm`, which already carries the batching patch). Pin provenance in `tui/vendor/NOCTERM_VERSION` (upstream SHA + patch list).
- **State architecture**: one immutable `WallState` (workspaces, sessions, focus, layer, overlay, per-tile scroll state) + a store that applies events (SSE, key, PTY lifecycle) — mirrors the daemon-is-authoritative rule; every render is a pure function of `WallState` + live terminal buffers. No state is persisted TUI-side in p8 except `~/.garage/tui.json` for the focused workspace.
- **API client**: plain `dart:io` HttpClient against `127.0.0.1:4747`; SSE parsed by a small line reader (two event types: `status`, `sessions`). No auth needed (daemon allows no-Origin requests). Poll fallback: refetch sessions every 5 s if SSE drops, with reconnect backoff.
- **Tiles**: `TerminalXterm` + `PtyController('tmux', ['attach','-t','=<id>'], env: TMUX removed)`. All key handling through our `onKeyEvent` (consume everything; never fall through to the framework's lossy default). Reattach loop: on PTY exit, if the session is still listed live, `restart()` after 500 ms (max 4 tries, then placeholder).
- **Encoder**: lift `encodeKey` from the spike verbatim, then extend (F-keys, Ctrl+arrows) and unit-test against a byte table. It is the single choke point for gate-2 fidelity.
- **Framework patches carried in the fork** (each with a regression test in `tui/test/`):
  1. batching: `\n`/`\r` never printable, runs < 4 chars stay individual events (exists from spike);
  2. debug-key: disable the hardcoded Ctrl+G intercept → the disengage chord is **Ctrl+G** per the mockup;
  3. Shift+Tab drop: reproduce in a harness, fix in the fork (parser handles `ESC[Z` in isolation — the live-app drop must be found; budgeted as its own task).
- **Scroll freeze**: our own render path over the xterm buffer — on freeze, record `anchor = totalLines`; render window `[anchor - viewHeight - pages*viewHeight, …]`; live tail resumes when the window reaches `totalLines`. Do not use `TerminalXterm`'s relative `scrollOffset`.
- **Daemon message field**: `status.js` store gains `message` (set by `hooks.js` on Notification transitions, cleared on any transition away from needs-input); `sessions.js` includes it in the listing. ~30 lines + tests.
- **Rendering budget**: fps15 cap set at startup; `stty -ixon -ixoff` before `runApp`, restored on exit. Startup ordering: daemon health check → sessions fetch → first frame → PTY attaches (staggered 50 ms apart to keep the first frames cheap).
- **Testing strategy**: pure logic (encoder, salience sort, layer state machine, scroll anchor math, SSE parser) via `dart test`; framework patches via nocterm's tester; end-to-end via the spike's tmux-driven harness pattern (send-keys + capture-pane assertions) in `tui/test/e2e/`, runnable by `npm test` alongside the existing suite. Phase ends with real-system verification recorded in `verification.md` (project convention).

**Alternatives considered**: ratatui/Rust (higher ceiling, slower iteration for this team — rejected after the spike passed all gates); reusing the `/term` WS bridge (adds a hop and nests tmux clients — rejected); tmux-controller-only without embedded tiles (loses the live grid, pillar 2 — rejected).

## Risks / Trade-offs

- [Shift+Tab live drop remains unexplained] → dedicated task with a repro harness before UI work depends on it; worst case, map Shift+Tab from the kitty-protocol path (host terminals that support it) and document the legacy-terminal gap.
- [Single-isolate saturation at larger terminals] → fps15 + staggered attach + dirty-tile rebuild task; measure in e2e with a 250×70 host before calling the phase done.
- [Vendored fork drifts from upstream] → `NOCTERM_VERSION` file + patches kept as three narrow diffs; upgrading = re-apply three patches.
- [Ctrl+G patch changes framework behavior] → patch only disables the debug toggle when a flag is set by the embedding app.
- [Coalescing under extreme load could still merge >3-char typing into paste] → acceptable: bracketed paste into Claude Code inserts text without submitting; documented behavior.

## Open Questions

- None blocking. Chord fallback if Ctrl+G patch misbehaves: Ctrl+Q (spike-proven).
