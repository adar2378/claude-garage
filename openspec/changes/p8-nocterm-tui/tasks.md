# Tasks: p8-nocterm-tui

## 1. Daemon: needs-input message

- [x] 1.1 Record Notification message in the status store on hook-sourced needs-input transitions; clear on any transition away (`daemon/src/hooks.js`, `daemon/src/status.js`)
- [x] 1.2 Expose `message` (or null) on `GET /api/sessions` entries (`daemon/src/sessions.js`)
- [x] 1.3 Unit tests: message set by hook, null for poller-sourced needs-input, cleared on answer, present in listing shape (`daemon/test/`)

## 2. TUI scaffold and vendored fork

- [x] 2.1 Create `tui/` Dart package (pubspec, analysis options, `bin/garage_tui.dart` stub that renders a hello-wall frame and exits cleanly, restoring the terminal)
- [x] 2.2 Vendor nocterm from the spike (`tui/vendor/nocterm` with the batching patch), add `dependency_overrides`, write `tui/vendor/NOCTERM_VERSION` (upstream version + patch list)
- [x] 2.3 Startup/shutdown plumbing: `stty -ixon -ixoff` + restore, fps15 cap, daemon health check with actionable error when daemon absent
- [x] 2.4 Regression test for the batching patch (chunked Enter+chars stay individual events) using nocterm's test framework

## 3. API client and wall state

- [x] 3.1 Daemon client: sessions/workspaces fetch, spawn, SSE subscription with reconnect backoff and 5s poll fallback (`tui/lib/api/`)
- [x] 3.2 `WallState` + store: immutable state, event application (SSE, focus, layer, overlay), salience sort lifted from `ui/src/lib/groups.js` semantics (`tui/lib/state/`)
- [x] 3.3 Unit tests: salience ordering (stable, blocked-first at both levels), SSE line parser, reconnect state machine

## 4. Key routing and encoder

- [x] 4.1 Port the spike encoder to `tui/lib/input/encode_key.dart`; extend with F1–F12 and Ctrl+arrows; table-driven unit tests for every sequence in the tui-key-routing spec
- [x] 4.2 Layer state machine (garage/engaged/overlay) with the keys-target chip model; garage bindings (`1-9 [ ] a A n N Enter ? q`); unit tests for layer transitions
- [x] 4.3 Paste path: synthetic Ctrl+V recovery via ClipboardManager → bracketed paste to PTY; real bracketed paste forwarding; tests
- [x] 4.4 Vendored patch: flag-gated disable of the Ctrl+G debug intercept; switch disengage chord to Ctrl+G; regression test

## 5. Tiles and grid

- [x] 5.1 Tile widget: TerminalXterm + tmux-attach PtyController (TMUX stripped), title bar (glyph, label, branch, elapsed), engaged/focused/blocked border treatments
- [x] 5.2 Reattach loop (PTY exit + session still live → restart with backoff, max 4) and restorable/dead placeholders
- [x] 5.3 Grid layout: `ceil(sqrt(n))` math (port `ui/src/lib/layout.js` buildDefault semantics), 6-tile cap with overflow swap-in on focus; staggered attach on startup; unit tests for layout + cap
- [x] 5.4 Rail + strip: workspace tree with sessions and glyphs, per-workspace amber dots, blocked count, keys-target chip, done-fade after 2 minutes

## 6. Triage

- [x] 6.1 `a` jump: longest-waiting blocked session, cross-workspace, overflow swap-in, lands engaged; strip notice when none
- [x] 6.2 Triage queue overlay: rows (identity, waiting time, daemon `message`), j/k/Enter/Esc, sorted by waiting time
- [x] 6.3 Escalation: terminal bell on new needs-input transition, OSC title with blocked count, `POST /api/ui/visibility` heartbeat
- [x] 6.4 Unit tests: jump target selection, queue ordering, bell-once-per-transition

## 7. Scrollback

- [x] 7.1 Frozen history render path over the xterm buffer (absolute anchor, page math), Shift+PageUp/Down while engaged, typing snaps to live
- [x] 7.2 Wheel: unengaged tile → frozen peek without engaging; engaged tile → forward to app; back-to-live affordance with new-line count
- [x] 7.3 Optional capture-pane seeding on attach (flag-gated); unit tests for anchor math and snap-back rules

## 8. Shift+Tab investigation

- [x] 8.1 Build a minimal repro harness (tmux-driven) for the live-app `ESC[Z` drop; identify the drop point in the vendored fork
- [x] 8.2 Fix in the fork with a regression test; if genuinely unfixable, document the gap and the kitty-protocol fallback in SPIKE.md and the spec deviation in verification.md

## 9. Packaging

- [x] 9.1 `claude-garage tui` subcommand: reuse prerequisite checks, start daemon if health check fails, exec the TUI binary, leave daemon running on exit
- [x] 9.2 Binary strategy: `npm run build:tui` (dart compile exe per-arch), lookup order (shipped binary → local build → actionable error); wire into package files
- [x] 9.3 README: TUI quick start section (do not touch other sections)

## 10. End-to-end verification

- [x] 10.1 tmux-driven e2e harness in `tui/test/e2e/` (spike pattern: send-keys + capture-pane): engage/disengage, Alt+arrow byte-exactness against a keyecho session, spawn, `a` jump, queue open/jump
- [x] 10.2 Load test: 5 stress sessions + keypress latency assertion (<50 ms) at 200×55 and 250×70
- [x] 10.3 Real-system verification with live Claude Code sessions across two workspaces; record results in `openspec/changes/p8-nocterm-tui/verification.md` (project convention: every phase ends with real-system verification)

## 11. p8.1 basic functionality

- [x] 11.1 Tile size → PTY propagation (vendored patch 8: constraint-sized renderer + running-transition re-push; the wrapped-text bug), registered in NOCTERM_VERSION with a regression test
- [x] 11.2 Maximize: `m` toggles the focused tile full-grid (WallState.maximizedSessionId + store toggle; focus change exits; siblings stay mounted, obscured), tested
- [x] 11.3 Restore: Enter/click on a restorable tile → POST /api/sessions/restore {id} with "restoring…" placeholder + strip-notice failures; `R` restores all restorable in the focused workspace via parallel per-id calls; engage still requires a live session
- [x] 11.4 Close/delete: `x` armed double-press (3s window, strip notice, any other key disarms) → DELETE /api/sessions/<id>; worktree-kept notice; restorable metas dropped via new `?meta=1` daemon query-param (daemon tests; web behavior unchanged)
- [x] 11.5 Workspace add: `w` overlay (OverlayKind.workspaceAdd, nocterm TextField, ~ expansion, client-side dir validation, web-UI name derivation) → PUT /api/workspaces, refetch, focus new workspace; Esc cancels
- [x] 11.6 Empty states: zero-workspace onboarding panel; empty-workspace n/N hint; never amber
- [x] 11.7 Help overlay + hints updated for w/x/m/R/Enter-on-restorable
- [x] 11.8 OpenSpec: ADDED requirements in tui-wall (PTY size, maximize, empty states) and tui-key-routing (p8.1 lifecycle bindings)
- [x] 11.9 Tests + verification: unit tests (maximize state, armed-close, path expansion/name derivation, restore-all selection, resize patch); scratch-port e2e (tui/test/e2e/run_p81.sh) incl. resize/list-clients check, restore round-trip, x-x close, w flow, empty states; run_click_smoke.sh made port/GARAGE_DIR-configurable and run; recorded in verification.md; dist binary rebuilt
- [x] 11.10 Engaged inner cursor (vendored patch 9: `TerminalXterm.showCursor` — solid inverse-video block at (cursorX, absoluteCursorY), DECTCEM-gated, suppressed while frozen/scrolled; tile passes engaged && !frozen), registered in NOCTERM_VERSION with a regression test (tui/test/terminal_xterm_cursor_test.dart) and live SGR-capture proof in tui/test/e2e/run_p82.sh
- [x] 11.11 Launcher stale-daemon gate: /api/health gains `version` + `pid` (daemon/src/health.js, tested); bin/garage.js compares against its own package version on both the tui and web launch paths and restarts a stale daemon by reported pid (lsof-by-port fallback for pre-upgrade daemons that report none) — "sessions are untouched" by design; startDetachedDaemon's daemon.log honors GARAGE_DIR; proven live in run_p82.sh (both the pid-less and pid-reporting stale shapes)
- [x] 11.12 p8.3 workspace remove: `X` armed double-press on the FOCUSED workspace → registry-only DELETE /api/workspaces/<name> (never ?sessions=kill; client.removeWorkspace); generic ArmedAction machine (armed_close.dart aliases it; x/X arms independent, cross-disarming); unregistered groups get the explanatory strip notice instead; live sessions resurface as the synthesized unregistered group after the refetch (store transition test); help overlay lists `X`
- [x] 11.13 p8.3 rail focus marker: reserved-column `▸` + bright/bold (never amber) on the focused session's rail row, tracking every focus change (digits, [/], clicks, a/A jumps, spawn, restore); one-line-per-row layout unchanged so railTargetAt click mapping needs no update; e2e smoke tui/test/e2e/run_p83.sh (marker follows ]/click, X-X remove → unregistered transition, X-on-unregistered notice) recorded in verification.md
- [x] 11.14 p8.4 kill-all removal confirm: while the `X` arm is live, `K` confirms via DELETE /api/workspaces/<name>?sessions=kill (client.removeWorkspace gains killSessions; ArmedAction stays generic — the `K` branch is caller-level in workspace_remove.dart's confirmKillTarget); arm notice appends `· K to also kill its <n> sessions` when n>0 live sessions (omitted at zero); confirm notice `removed <name> · killed <n> sessions` (failedSessions named); `K` outside an arm stays unbound (typing hint), X-X stays registry-only, cross-disarm unchanged; help overlay lists `X K`; unit tests tui/test/workspace_remove_test.dart; e2e tui/test/e2e/run_p84.sh (X-K kills both scratch sessions + unregisters, X-X leaves sessions alive, zero-session wording) recorded in verification.md
