# claude-garage

**Claude Code session manager — parallel sessions, grouped terminals, needs-input triage, diff review. A pit wall for your agents.**

## What it is

claude-garage is a local web app for running several Claude Code sessions across several projects at once, without losing track of which one needs you. A workspace rail groups sessions by project and sorts needs-input-first; a terminal grid shows every session in the focused workspace live and simultaneously; a diff pane and full-screen review mode cover what each session changed. It never owns your sessions — tmux does. Close the app, and nothing dies: every session stays a real `tmux` session, reachable from any terminal with `tmux attach -t garage/<workspace>/<label>`, whether or not the garage UI is running. If your Mac reboots and tmux itself dies, garage can bring the conversations back too (`claude --resume`), not just the panes.

## Install & run

Prerequisites:

- macOS
- [tmux](https://github.com/tmux/tmux) ≥ 3.2 — `brew install tmux`
- the [Claude Code CLI](https://docs.claude.com/en/docs/claude-code) (`claude`) installed and on `PATH`
- Node.js ≥ 20

Then, from anywhere:

```
npx claude-garage
```

This starts the daemon on `http://127.0.0.1:4747`, serves the UI from the same process, and best-effort opens the URL in your browser.

## Hook setup

By default garage detects "needs input" by polling `claude agents --json` every couple of seconds — it works, but it's coarse. For precise, instant detection, install Claude Code's own hooks — one click:

- Click **install hooks for me** in the banner the garage UI shows (it calls `POST /api/hooks/install`, which merges the hook entries into `~/.claude/settings.json` after saving a timestamped backup next to it; running it again is a safe no-op).

Prefer doing it by hand? `GET http://127.0.0.1:4747/api/hooks/snippet` returns the JSON to merge into `~/.claude/settings.json` yourself.

Without hooks installed, garage still works — status just lags behind the poller's interval instead of updating the instant Claude asks for input.

## Keybindings

| Key | Action |
|---|---|
| `1`–`9` | switch focused workspace to the Nth in rail order |
| `[` / `]` | cycle the focused terminal within the current workspace |
| `a` | jump to a session that needs input, in any workspace |
| `\` | split the focused terminal right (spawns a new session there) |
| `m` | maximize the focused terminal ⇄ restore the grid |
| `Tab` | changes pane: toggle list ⇄ diff |
| `j` / `k` | next / previous changed file |
| `r` | enter full-screen review mode |
| `Esc` | close the help overlay, else exit review mode |
| `v` | (review mode) mark file viewed, advance to next unviewed |
| `o` | open the selected file (or workspace root) in your editor |
| `Ctrl+\`` | blur the focused terminal — back to chrome-navigation mode, works even while a terminal has keyboard focus |
| `?` | toggle this keybinding help overlay |

All bindings except `Ctrl+\`` are suppressed while a terminal has keyboard focus — keystrokes go to Claude Code's pty instead.

## How it works

tmux is the source of truth for every session; garage is a thin, near-stateless viewer on top of it.

```
tmux  (persistence — source of truth)
  └─ small Node daemon  (spawn / list / bridge / diff / hooks)
       └─ browser UI  (React + xterm.js)
```

- **tmux** owns the actual processes. Sessions are named `garage/<workspace>/<label>`; `tmux ls` is the list API. Killing the garage UI, or the daemon, never touches tmux.
- **Daemon** — a thin Fastify process. It shells out to `tmux`/`git`/`claude` rather than re-implementing any of them, bridges each terminal over a WebSocket (`node-pty` ⇄ xterm.js), computes diffs on demand, and receives Claude Code's hook events. State it can't recompute (workspace → directory mapping, restore metadata) lives in one small file, `~/.garage/state.json`.
- **UI** — React + xterm.js, rendering the rail, the live multi-terminal grid, and the diff/review panes over that daemon's HTTP + WebSocket + SSE API.
- **Escape hatch**: garage owns orchestration and triage, not deep editing — jump to VS Code (`o`, or the per-workspace root button) or a plain terminal (`tmux attach`) whenever you want the full tool.

## Development

```
npm install
npm run dev
```

This runs the daemon (`:4747`) and Vite's dev server (`:5173`, proxying `/api` and `/term` to the daemon) concurrently. The workspace is a two-package npm workspace: `daemon/` (Fastify + tmux/git/hook integration) and `ui/` (the React app, built to `ui/dist` for the packaged `npx claude-garage` entrypoint).

---

Built through OpenSpec-driven phases (P0–P3), each verified end-to-end before moving on — see [`openspec/changes/archive/`](openspec/changes/archive/) for the full spec/design/task history of every phase.
