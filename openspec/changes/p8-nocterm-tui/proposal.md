# Proposal: p8-nocterm-tui

## Why

The browser UI is the adoption bottleneck: the terminal-native audience that wants tmux-owned sessions (the audience herdr captured at 21k★) won't live in a localhost tab, and the web UI cannot deliver verbatim key passthrough (the Alt+arrow bug). The nocterm spike (2026-08-30, `spikes/nocterm-wall/SPIKE.md`) passed all three gates — rendering performance, verbatim passthrough proven in a real Claude Code composer, scrollback mechanism — so the TUI is now buildable with known, bounded risk.

## What Changes

- New `tui/` Dart workspace: a nocterm-based full-screen terminal client over the existing daemon HTTP/SSE API. The daemon is reused verbatim; the TUI attaches tiles directly via `tmux attach` PTYs (no WS terminal bridge).
- Vendored nocterm fork (`tui/vendor/nocterm`, pinned) carrying the spike's input patches; spike learnings (fps15 cap, `stty -ixon`, raw key re-encoder, paste recovery, dead-PTY reattach) are baked in as requirements.
- The Wall: workspace rail + live tile grid (cap 6, overflow rail-only) + status strip, salience-sorted, amber reserved for needs-input — per the approved UX mockup (claude.ai artifact `7dc66760`).
- Three-layer key routing (garage / engaged / overlay) with an always-visible keys-target chip; `a` jump lands engaged.
- Triage queue overlay showing per-session waiting time and the Claude notification question text.
- Frozen local scrollback in tiles (absolute anchor + "back to live"), never tmux copy-mode.
- Daemon: store the latest Notification hook message per session and expose it in `GET /api/sessions` (small addition; both UIs benefit).
- Packaging: `npx claude-garage tui` launches the compiled TUI binary (per-arch dart compile), daemon auto-started if absent.

Out of scope for p8 (later phases): review mode + review comments, overview (`0`), fleet restart & resume, context meters, command palette, pit pet, worktree finish modal, web UI changes.

## Capabilities

### New Capabilities
- `tui-wall`: full-screen nocterm client — workspace rail, live tile grid over tmux-attach PTYs, status strip, salience ladder, tile lifecycle (spawn/reattach/dead-PTY recovery), fps cap.
- `tui-key-routing`: the three key layers, raw byte re-encoder (verbatim passthrough incl. Alt+arrows and paste), engage/disengage chord, keys-target chip.
- `tui-triage`: needs-input sorting in rail and strip badge, `a` jump (cross-workspace, lands engaged), `A` triage queue overlay with waiting time and question text.
- `tui-scrollback`: frozen local history view per tile (Shift+PageUp/Down and wheel), seeded from the tile buffer, absolute anchor, live-tail return.

### Modified Capabilities
- `session-status`: daemon additionally records the most recent needs-input notification message per session and exposes it via `GET /api/sessions`.
- `packaging`: the npm package gains a `tui` subcommand and ships/builds the compiled TUI binary; daemon startup is shared between web and TUI entry points.

## Impact

- New code: `tui/` (Dart, nocterm vendored fork; spike code in `spikes/nocterm-wall` is the reference, not the product).
- Daemon: small additions to `daemon/src/hooks.js`, `daemon/src/status.js`, `daemon/src/sessions.js` (message field); no breaking API changes.
- Packaging: `bin/` launcher and npm scripts; Dart SDK becomes a build-time dependency (not a user install requirement if binaries are shipped per release).
- Web UI: untouched.
- Tests: daemon unit tests extended for the message field; TUI logic (encoder, salience sort, layer state machine, scroll anchor) unit-tested with nocterm's test framework.
