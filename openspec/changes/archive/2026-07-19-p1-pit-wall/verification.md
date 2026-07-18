# P1 e2e verification — 2026-07-19

Environment: macOS, Node v22.22.3, tmux 3.7b, Claude Code 2.1.214. Two workspaces (`garage-dev` → repo, `sandbox` → scratch project dir), four live claude sessions. Hooks installed into `~/.claude/settings.json` (backup: `~/.claude/settings.json.backup-pre-garage-2026-07-19`); sandbox sessions respawned post-install since hook config is captured at session start.

## 4.1 Hook precision with a real permission prompt

Real flow in `garage/sandbox/alpha` (manual permission mode): prompt asking Claude to run `touch e2e-marker.txt` → Bash approval dialog.

| Timestamp (epoch s) | Event |
|---|---|
| …744.964 | Enter sent to alpha |
| …745.677 | SSE `working` (poller pid-join, **0.7s**) |
| …748.113 | transient `idle` blip between turn phases (cosmetic, noted) |
| …754.024 | SSE `needs-input` via Notification hook — effectively simultaneous with the dialog rendering, well under the 2s budget |

Approval was performed **from the pit wall** (click into terminal, Enter) — command executed (`e2e-marker.txt` created), `Stop` hook fired.

## 4.2 Full pit-wall scenario (Playwright)

- Rail: both workspaces with index hints, session counts, nested sessions with glyphs; ● amber on blocked alpha. Screenshot: `p1-pit-wall-needs-input.png` (Playwright output dir).
- **Two terminals of one workspace visibly live at once** — alpha rendering its permission dialog while beta rendered its own TUI, same view, zero window switches. The core differentiator, on screen.
- Keybindings: `2` switched grid to garage-dev ✓; `]` cycled cell focus alpha→beta (amber ring moved, verified via DOM) ✓; `a` from garage-dev jumped across workspaces to blocked alpha ✓.
- Suppression: with DOM focus inside a terminal, typed `2` went to the pty input line (verified via `tmux capture-pane`) and did NOT switch workspaces ✓. Clicking header blurs back to chrome-navigation mode ✓.

## 4.3 macOS notification

`needs-input` fired while no visible page → exactly **one** daemon log line `needs-input notification fired` + osascript invocation; count stayed 1 across ~1 min of blocked state and multiple poll ticks (edge-triggered, no repeats) ✓.

## 4.4 Needs-you-first ordering, live

- While alpha was blocked: sandbox listed **above** garage-dev; alpha before beta within the group ✓ (bubble-up).
- After approval cleared the block: garage-dev reclaimed position 1 **without reload** (SSE-driven reorder) ✓ (un-bubble).
- Status glyphs updated live throughout (● → ◐ → ✓/○ transitions observed in rail and cell title bars).

## Bug found & fixed during verification

**Poller stomped `done` → `idle`.** The Stop hook set `done`, then the next 2s poll tick (agents-json reports `idle`) overwrote it, defeating the 2-minute decay. Fix in `poller.js`: coarse `idle` no longer downgrades either precision state (`needs-input` or `done`). Verified: after a Stop hook, `done` persisted across ≥3 poll ticks.

## Product discovery (deferred, recorded)

Claude Code's **folder-trust dialog** on first run in a new directory is a real needs-input state that neither hooks (not yet initialized) nor `agents --json` (reports idle) can see. New sessions in never-trusted dirs silently block. Candidate fix for a later change (e.g. treat a fresh session with no signals + no agents-json entry as needs-attention).

## Verdict

**P1 gate: PASS.** The pit wall does the thing no competitor does: grouped live terminals + precise needs-input triage + attention routing off-screen, verified against real Claude Code sessions end-to-end.
