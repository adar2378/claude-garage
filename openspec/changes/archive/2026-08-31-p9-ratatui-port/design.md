# Design: p9-ratatui-port

## Context

p8 shipped a complete, verified TUI on nocterm/Dart, but the framework needed 9 vendored patches and its upstream path is closed. The ratatui spike (`spikes/ratatui-wall/SPIKE.md`) passed all gates with pinned published crates and materially better numbers. p8's OpenSpec specs are framework-neutral behavior contracts, and its e2e harnesses drive a compiled binary through tmux — together they make this a translation with an executable acceptance gate, not a rewrite.

## Goals / Non-Goals

**Goals:**
- Feature and behavior parity with the shipping Dart TUI, proven by the existing harnesses passing unchanged (binary path parameterized).
- Zero forks: ratatui, crossterm, portable-pty, tui-term, vt100 at pinned published versions.
- Encode both spike findings as tests from day one (teardown injection; capture-pane-only scrollback).

**Non-Goals:**
- No new features (context meters, views, `t` standalone — they come after parity, in Rust).
- No daemon or web UI changes.
- No Linux/Windows yet (keep the code portable; macOS arm64 is the gate).
- No changes to the frozen nocterm fork or spikes.

## Decisions

- **Crates** (from the spike): ratatui + crossterm (input — modifiers survive coalescing natively), portable-pty (TIOCSWINSZ-correct resize), tui-term 0.3.4 + vt100 0.16.2 (Turborepo perf fixes verified upstreamed; tui-term's O(rows) per-cell indexing measured negligible at tile sizes — first optimization candidate if 250×70 ever regresses). ghostty-vt FFI is the documented escalation, unused.
- **Workspace layout**: `wall/` (cargo workspace; binary `garage-wall`). Module map mirrors `tui/lib/`: `api/` (daemon client + SSE), `state/` (wall state, store, salience, armed actions, workspace form), `input/` (encoder, paste), `ui/` (tiles, registry, rail, strip, overlays, hit targets, scroll), `escalation`. Port order follows p8's dependency order; every Dart unit test translates to a Rust test before or with its module (the Dart suite is the porting spec at function level).
- **Runtime**: tokio; PTY readers and the SSE stream as tasks feeding one event channel; a single state-owning loop applies events and draws (ratatui immediate mode) — same "daemon is authoritative, render is pure" shape as p8.
- **Teardown discipline** (spike finding 2): a `TileClient` type owns attach lifecycle; `tmux detach-client -s` always precedes master close or kill; Drop is detach-first. Regression test = the recorder-pane byte-proof from the spike.
- **Scrollback** (spike finding 1): frozen view = one `capture-pane -e` snapshot at an absolute `#{history_size}` anchor + page offset; live view = the vt100 screen. The Dart absolute-anchor math ports as the paging model over the snapshot.
- **Parity gate**: `tui/test/e2e/run_e2e.sh`, `run_p81.sh`–`run_p84.sh` take the binary path as an env/arg (small harness edit, no check changes) and must pass against `wall`'s binary; plus the real-Claude-composer smoke (spike caveat: not yet retested on Rust) and the load-latency numbers at both sizes recorded in verification.md.
- **Launcher transition**: lookup order gains the Rust dist binary ahead of Dart; the Dart path and `tui/` are deleted in the final task group only after verification.md records full parity (one commit, easy to revert).

## Risks / Trade-offs

- [tui-term rendering cost at larger terminals] → measured acceptable in the spike; keep the keylog latency check in e2e so regression is caught, optimize indexing only if it fails.
- [Behavior drift the harnesses don't cover] → the harnesses grew through four user-testing waves and cover every reported issue; remaining drift risk is visual polish — do a side-by-side screenshot pass (Dart vs Rust) before removal.
- [Claude Code composer nuances on Rust] → explicit early task; byte-exactness at the PTY already proven.
- [Two TUIs in-tree during the port] → bounded by the final removal group; launcher prefers Rust only once parity is recorded.

## Open Questions

- None blocking.
