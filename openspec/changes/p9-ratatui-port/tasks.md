# Tasks: p9-ratatui-port

## 1. Scaffold and foundations

- [x] 1.1 `wall/` cargo workspace (binary `garage-wall`): pinned deps from the spike, clippy config, startup/shutdown plumbing (stty flags + full restore), daemon health gate honoring GARAGE_TUI_PORT/GARAGE_PORT
- [x] 1.2 `TileClient` PTY lifecycle type with detach-first teardown; recorder-pane injection regression test (spike finding 2)
- [x] 1.3 Lift the spike's wall (`spikes/ratatui-wall/src/main.rs`) as reference; establish the event-channel + single-state-loop runtime shape

## 2. API client and state

- [x] 2.1 Daemon client + SSE (reconnect backoff, poll fallback) — port `tui/lib/api/`, tests
- [x] 2.2 Wall state + store + salience + armed actions + workspace form — port `tui/lib/state/` with its full unit-test suite translated
- [x] 2.3 Grid math, hit targets — port with tests

## 3. Input

- [x] 3.1 Encoder (every sequence row from the tui-key-routing spec) from crossterm events; table tests ported
- [x] 3.2 Paste path (bracketed forwarding); coalesced-input e2e check retained
- [x] 3.3 Key layers + garage bindings incl. p8.1–p8.4 lifecycle keys (n N x X K w m R Enter a A [ ] ? q, Ctrl+G)

## 4. UI surfaces

- [x] 4.1 Tiles (tui-term rendering, engaged cursor, title bars, placeholders, reattach loop, resize-on-layout via TIOCSWINSZ)
- [x] 4.2 Rail (salience, ▸ focus marker, amber discipline), strip (tabs, badge, chip, notices), help overlay
- [x] 4.3 Triage queue overlay + escalation (bell, OSC title, visibility heartbeat)
- [x] 4.4 Workspace-add overlay; empty states; mouse clicks (tiles/rail/badge/overlays) + wheel
- [x] 4.5 Maximize; frozen scrollback via capture-pane anchor (spike finding 1) with back-to-live affordance

## 5. Packaging

- [x] 5.1 `npm run build:tui` → cargo build of `wall/`; launcher lookup order: Rust dist binary → cargo self-build → Dart fallback (transition) → actionable error
- [x] 5.2 README ## TUI section: no user-visible changes needed beyond the build note; verify

## 6. Parity verification

- [x] 6.1 Parameterize the binary path in `tui/test/e2e/run_*.sh` (no check changes); all five suites green against the Rust binary — 178/178 across all six suites (incl. run_click_smoke.sh); 2 failures found → fixed (engaged-cursor-on-empty-cell in wall/, ECMA-48 extended-color parse in the p82 SGR parser); launcher gained the GARAGE_TUI_BIN test hook
- [x] 6.2 Real Claude Code composer smoke (trust dialog, typing, Alt+arrow word-jump via inner cursor) — the spike's open caveat — closed against claude 2.1.251, all checks pass, no prompt submitted
- [x] 6.3 Load-latency + CPU at 200×55 and 250×70; side-by-side visual pass vs the Dart wall; record everything in `openspec/changes/p9-ratatui-port/verification.md` — 5 ms medians, ~5× less CPU / ~8× less RSS than Dart; visual parity with 3 cosmetic drifts reported

## 7. Removal at parity

- [ ] 7.1 Launcher prefers Rust unconditionally; delete `tui/` (Dart client), the nocterm git dependency, and the Dart build path — one revertable commit, only after 6.x is fully recorded
- [ ] 7.2 Update NOCTERM_VERSION pointer / docs to reflect the frozen-history status of the fork
