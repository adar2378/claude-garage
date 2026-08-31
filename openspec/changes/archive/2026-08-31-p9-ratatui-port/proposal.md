# Proposal: p9-ratatui-port

## Why

The nocterm foundation required 9 vendored patches in one phase, and the upstream path is closed (user decision) — we would be maintaining a fork of an untrusted framework forever. The ratatui spike (`spikes/ratatui-wall/SPIKE.md`, 2026-08-31) passed all three gates with zero patches and zero vendoring: 6ms latency, ~2% CPU (vs 13%), 5.9MB RSS (vs 70MB), byte-exact passthrough including the coalesced-input class that cost nocterm three patches. Rust/ratatui is also the ecosystem where terminal-tool contributors live, which serves the community-plugin strategy.

## What Changes

- New `wall/` Rust workspace: the TUI ported to ratatui + crossterm + portable-pty + tui-term/vt100 (pinned published crates — no forks). Behavior contracts are p8's existing specs; the acceptance gate is the existing tmux-driven e2e harnesses (`tui/test/e2e/run_*.sh`) passing unchanged against the Rust binary.
- The Dart TUI (`tui/`) keeps shipping until the Rust binary passes every harness; it is removed in this change's final task group only after parity is recorded in verification.md.
- Launcher: `claude-garage tui` gains the Rust binary as the primary lookup (fallback order updated); `npm run build:tui` becomes a cargo build.
- Spec-level behavior changes (small, spike-driven): the 15fps render cap is no longer mandated (it was a nocterm workaround); tile scrollback is explicitly capture-pane-seeded (tmux attach clients live on the alt screen — the emulator buffer never fills); PTY teardown must never inject bytes into panes (`tmux detach-client` before master close).
- The nocterm fork and `spikes/` stay frozen as history.

## Capabilities

### New Capabilities

_None — the product surface is unchanged; this is a stack port._

### Modified Capabilities

- `tui-wall`: performance requirement drops the mandated 15fps cap (latency/CPU budgets stay); new teardown requirement — closing/quitting never injects bytes into attached panes (SIGHUP/EOF injection class).
- `tui-scrollback`: history depth requirement reworded — frozen view is seeded from `tmux capture-pane` at an absolute history anchor (the only mechanism for alt-screen attach clients), not from a live emulator buffer.
- `packaging`: TUI binary strategy becomes the Rust build (cargo, per-platform); Dart SDK ceases to be the self-build dependency.

## Impact

- New code: `wall/` (Rust). Ported logic mirrors `tui/lib/` module-for-module; the encoder, salience sort, layer machine, armed actions, and scroll anchor are direct translations of unit-tested Dart code.
- Unchanged: daemon, web UI, all p8 spec behavior not listed above, e2e harnesses (binary path becomes a parameter).
- Removed at parity: `tui/` (Dart client), the nocterm git dependency, `build:tui` dart path.
- Toolchain: cargo 1.94.1 (present); CI/release later gains a cargo build step.
