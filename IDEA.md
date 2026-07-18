# claude-garage

**Pit crew for your Claude Code sessions. Grouped terminals, live diffs, one screen.**

> Status: idea / pre-build (2026-07-18). Interactive UX mock: `mock/session-deck.html` (open in browser).

---

## Problem

Running multiple Claude Code sessions across multiple projects means juggling terminal windows and VS Code windows. There is no single place to see:

- which sessions exist, per project
- which session is **blocked waiting for my input** (the real pain — not window count, attention routing)
- what each session **changed** (diffs), reviewable without hunting

## Concept

A local web app that is a **thin viewer over real terminals**. It never owns the sessions — tmux does. Close the app, nothing dies.

Three panes + status bar (TUI aesthetic, browser rendering):

1. **Workspaces (left)** — projects as groups, sessions nested under each (branch, status, diff stat). Sorted needs-you-first. States: `●` needs input / `◐` working / `✓` review ready / `○` idle.
2. **Terminal grid (center)** — ALL terminals of the focused workspace visible at once, stacked. Click or `[` `]` to focus one; focused terminal drives the right pane.
3. **Changes (right)** — diff + file list scoped to the focused session only. Glance-level.

Plus **full-screen review mode** (`r`) for bigger diffs — file rail with GitHub-PR-style viewed checkmarks (`v`), continuous full-width diff, `j/k` between files.

### Keybindings

| Key | Action |
|---|---|
| `1–9` | switch workspace |
| `[` `]` | cycle terminal within workspace group |
| `a` | jump to whichever session (any workspace) is blocked on input |
| `j/k` | next/prev changed file |
| `Tab` | diff ⇄ files |
| `r` / `Esc` | enter / exit full-screen review mode |
| `v` | mark file viewed (review mode), auto-advance to next unviewed |
| `o` | open current file in VS Code (`code --goto file:line`) |

### VS Code integration (escape hatch, not a cage)

- Per-workspace button: **open project root** → `code <dir>`
- Per-file in review mode: `o` → `code --goto <file>:<line>`
- Philosophy: garage owns *orchestration + triage*; VS Code stays the *deep review / editing* tool. Jump deliberately, one keystroke, instead of living in five windows.

---

## Architecture

```
tmux  (persistence — source of truth)
  └─ small Node daemon  (spawn / list / bridge / diff / hooks)
       └─ browser UI  (React + xterm.js — the mock, made real)
```

### Terminals are real terminals that stay

- "New session in kowboy" → `tmux new-session -d -s garage/kowboy/<label> -c <project-dir> claude`
- UI terminal panes = xterm.js attached via `node-pty` running `tmux attach`
- On startup: `tmux ls`, filter `garage/` prefix → rebuild the whole deck. Workspace grouping comes from session names: `garage/<workspace>/<label>`
- **Escape hatch:** any session reachable from any plain terminal — `tmux attach -t garage/kowboy/checkout` in iTerm/VS Code drops into the same live session (tmux mirrors multiple attaches natively)
- Daemon is almost stateless — tmux IS the registry. Small JSON (`~/.garage/state.json`) only for workspace→dir mapping + restore metadata

### Daemon endpoints (sketch)

- `GET  /api/sessions` — parse `tmux ls` (garage/ prefix) + status map
- `POST /api/sessions` — `{workspace, label}` → tmux new-session at correct dir, run `claude`
- `DELETE /api/sessions/:id` — tmux kill-session
- `WS   /term/:id` — node-pty ⇄ xterm.js bridge (tmux attach)
- `GET  /api/diff/:workspace` — `git status` + `git diff` for that project
- `POST /api/hooks/claude` — receives Claude Code hook events → status
- `POST /api/open-editor` — `{dir}` or `{file, line}` → spawn `code` CLI
- Bind 127.0.0.1 only.

### "Needs input" detection — no output parsing

Claude Code hooks in `~/.claude/settings.json`:

- `Notification` hook (fires when Claude waits for input) → POST to daemon → status `wait` (amber ●)
- `Stop` hook (fires when Claude finishes) → status `done` (✓) + trigger diff refresh
- Hook payload includes project dir → maps to session

### Reboot recovery

tmux dies on reboot. On next launch: daemon sees sessions gone → offers **restore** → recreate each tmux session from state.json and run `claude --resume` (conversation comes back too).

---

## Naming & SEO (decided: claude-garage)

- Winning ecosystem pattern: `claude-<short evocative noun>` (claude-mem 87k★, claude-code-router 35k★, claude-hud 26k★, claude-squad 8k★). "claude" in the name catches every `claude ...` search — the Claude Squad effect.
- **claude-garage: fully clean** — npm free, GitHub clean, no trademark noise. Metaphor: the garage is where the team works on cars mid-race; sessions *live* here.
- Rejected: `claude-pitstop` (Enfocus PitStop = 1993 software trademark; EdwinVW/pitstop 1.1k★ .NET sample), `claude-tower` (3 existing repos), `claude-deck/fleet/grid/dock/crew/lanes` (all squatted, some by near-competitors), bare `helm/pitwall/termdock` (owned/contested).
- Repo: `devmonks-co/claude-garage` (or personal). Bin: `npx claude-garage`, command `garage`. Domain: claudegarage.dev or garage.devmonks.co.
- SEO rule: brand word alone never carries keywords — repo description / README H1 / npm description all include "Claude Code session manager", "parallel sessions", "grouped terminals", "diff review".

## Competitive landscape (2026-07)

| Tool | Stars | Note |
|---|---|---|
| vibe-kanban | 27k | company shut down Apr 2026 — 27k users orphaned, community-run |
| claudecodeui | 12.7k | web/mobile remote control focus, not grouped workspace terminals |
| claude-squad | 8k | tmux TUI — proves our architecture; no rich diff review |
| ccmanager | 1.2k | TUI session manager |
| agent-deck | 530★ | TUI, multi-agent |
| tmux-claude-session-manager (craftzdog) | 314★ | tmux per project — architecture validation, no UI |
| crystal | 3k | deprecated Feb 2026 → paid Nimbalyst |

**Open slot:** grouped live terminals per workspace + first-class diff review + needs-input triage, in a web UI, on persistent tmux. Nobody combines all four. Timing: two popular tools just died; users are shopping.

## Pre-build gate (do BEFORE writing code)

Spend ~1h actually running: **claude-squad**, **claudecodeui**, **agent-deck**, craftzdog's tmux script, and the official **Claude Code desktop app**. If one is ≥90% of the vision → contribute instead of build. Gate artifact: a short gap table in this folder.

## Build phases

| Phase | Scope | Gate (binary) |
|---|---|---|
| P0 | daemon + spawn/list + single terminal attached in browser | kill browser tab → reattach → session alive with history |
| P1 | workspace grid + `garage/` naming + hook-based status + `a` triage | two workspaces, four sessions, amber dot fires from a real Notification hook |
| P2 | diff panel + review mode + open-in-VS Code | review a real 4-file diff start-to-finish without touching iTerm |
| P3 | reboot restore (`claude --resume`) + keybindings polish + `npx` packaging | fresh clone → `npx claude-garage` works on a second machine |

Rough effort: P0–P2 ≈ 2–3 weekends. OSS candidate for devmonks-co if it survives the pre-build gate.

## Stack

- **tmux** — session persistence
- **Node daemon** — Fastify/Express + ws, `node-pty`
- **UI** — React + Vite + xterm.js; design language from the mock (dark TUI aesthetic, mono, amber focus)
- **Diff rendering** — `parse-diff` + shiki (or diff2html) — syntax-highlighted, side-by-side possible later
- macOS first; Linux should work free (tmux/pty are portable), Windows out of scope
