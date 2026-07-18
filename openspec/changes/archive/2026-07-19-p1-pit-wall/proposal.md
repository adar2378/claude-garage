# Proposal: p1-pit-wall

## Why

P0 proved one terminal survives in the browser; P1 builds the actual product thesis: the pit wall. GAP.md verified that every competitor is a *switcher* (one terminal visible, jump between them) and none routes attention — "which session is blocked on me right now" is an unsolved problem across the field. P1 delivers the two differentiators in one change: all terminals of a workspace visible at once, and needs-you-first triage that works even when the pit wall isn't visible (macOS notifications).

## What Changes

- **Workspace registry**: first daemon state beyond tmux — `~/.garage/state.json` mapping workspace name → project directory (needed to spawn sessions into a workspace from the UI, and later for P3 restore). tmux remains the source of truth for *sessions*; the registry only remembers *directories*.
- **Session spawn via workspace** (**BREAKING** for the P0 API): `POST /api/sessions` takes `{workspace, label}` and resolves the directory from the registry; a separate endpoint registers workspaces. No external consumers exist, so breakage is theoretical.
- **Workspace rail (left pane)**: workspaces as groups with their sessions nested (label, status glyph), built from `garage/` session-name parsing + registry; sorted needs-you-first.
- **Terminal grid (center)**: ALL sessions of the focused workspace rendered as live xterm.js terminals simultaneously, stacked; click or `[` `]` to focus one.
- **Session status detection**: per-session state — `●` needs input / `◐` working / `✓` done / `○` idle. Mechanism (Claude Code hooks vs `claude agents --json` polling) is decided by a spike task; the spec defines states and latency budget, not the mechanism.
- **Attention routing**: needs-you-first sorting everywhere, amber indicators, `a` key jumps to the blocked session (any workspace), and a macOS notification fires when a session flips to needs-input while the page is hidden.
- **Keybindings**: `1–9` switch workspace, `[` `]` cycle terminal in group, `a` jump to blocked.

Out of scope (later changes): diff panel and review mode (P2), VS Code jump (P2), reboot restore and packaging (P3), tmux-native deck layout (deferred per IDEA.md surface decision).

## Capabilities

### New Capabilities
- `workspace-registry`: registering/listing workspaces (name → directory), persisted in `~/.garage/state.json`; validation and collision rules.
- `session-status`: per-session state model (needs-input / working / done / idle), detection mechanism contract, latency budget, and the status API/stream the UI consumes.
- `pit-wall-ui`: workspace rail, simultaneous multi-terminal grid, focus model, keybindings, needs-you-first ordering, and macOS notification on needs-input.

### Modified Capabilities
- `session-lifecycle`: spawn contract changes — `POST /api/sessions` resolves the project directory from the workspace registry instead of accepting a raw `dir` in the body.

## Impact

- Daemon: new registry module + endpoints, status tracking (hook receiver and/or poller), possibly a WS/SSE status stream to push updates to the UI.
- UI: full pit-wall layout replaces the P0 single-terminal page; multiple concurrent WS terminal connections (one pty per visible terminal — acceptable at focus-driven scale, ≤ ~6 visible).
- User setup: if the spike chooses hooks, garage must install/instruct hook config in `~/.claude/settings.json` — setup burden is a spike evaluation criterion.
- New dependency (daemon): none expected; macOS notifications via `osascript`/`terminal-notifier` subprocess.
- Specs: `session-lifecycle` gets a delta; three new capability specs.
