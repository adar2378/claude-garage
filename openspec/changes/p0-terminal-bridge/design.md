# Design: p0-terminal-bridge

## Context

Greenfield. IDEA.md fixes the architecture: tmux is the source of truth for sessions; a small Node daemon bridges tmux to a browser UI; the UI is a thin viewer (React + Vite + xterm.js). GAP.md verified no existing tool offers real surviving terminals in a web UI — this change validates that pipeline end-to-end with one session before any multi-session UI is built. Target machine: macOS with tmux ≥ 3.2, Claude Code CLI, Node ≥ 20.

## Goals / Non-Goals

**Goals:**
- Prove the tmux ⇄ node-pty ⇄ WebSocket ⇄ xterm.js pipeline with a real interactive Claude Code session.
- Establish the `garage/<workspace>/<label>` naming contract that later changes (grouping, status) build on.
- Prove session survival: browser tab death, daemon death, and plain-terminal `tmux attach` all leave the session intact.

**Non-Goals:**
- No workspace grid, no status detection, no diff panel, no restore, no packaging (later changes).
- No auth (loopback-only bind is the P0 security model).
- No Windows support; Linux works only incidentally.

## Decisions

**D1 — tmux session name IS the registry; no database.**
`garage/<workspace>/<label>` encodes everything the daemon needs; `tmux ls -F` is the list API. Alternative considered: SQLite metadata store (agent-deck's approach) — rejected because a second source of truth can drift from tmux and breaks the "daemon is almost stateless" principle. The only file state (workspace→dir mapping for restore) is deferred to P3. Constraint: tmux forbids `:` and `.` in session names — labels/workspaces are validated to `[a-z0-9-]+`.

**D2 — Fastify + `ws` for the daemon.**
Fastify over Express for built-in schema validation and cleaner async; plain `ws` (not `@fastify/websocket`) for the terminal socket so the pty bridge owns the raw socket without framework indirection.

**D3 — one `node-pty` running `tmux attach -t <name>` per WebSocket connection.**
Alternative considered: tmux control mode (`-CC`) multiplexing all sessions over one process — more efficient but a large protocol surface; deferred until it hurts. With attach-per-socket, tmux natively mirrors multiple attaches, so the iTerm escape hatch and a second browser tab work for free. On WS close the pty is killed — that detaches the client, never the session.

**D4 — resize flows client → pty; tmux reconciles.**
xterm.js `fit` addon reports cols/rows on a `resize` control message; daemon calls `pty.resize()`. tmux sizes the session to the smallest attached client (`aggressive-resize` on for the garage group later if needed).

**D5 — WS protocol: binary frames = terminal bytes, JSON text frames = control.**
Output/input are raw bytes (no base64 overhead). Control messages (`{type:"resize",cols,rows}`) are text frames. Alternative: socket.io — rejected, unnecessary dependency for a loopback pipe.

**D6 — npm workspaces monorepo: `daemon/` + `ui/`.**
Vite dev server proxies `/api` and `/term` to the daemon (single origin in the browser), so CORS never enters the picture. Production serving of built UI by the daemon is a P3 concern.

**D7 — spawn command: `tmux new-session -d -s "garage/<ws>/<label>" -c <dir> claude`.**
Detached from birth — the daemon never owns the process. If `claude` exits, the tmux session ends and disappears from the list; that honestly reflects reality (no `remain-on-exit` magic in P0).

## Risks / Trade-offs

- [node-pty is a native module; build breakage across Node/macOS versions] → pin Node ≥ 20 LTS in engines, commit a lockfile, document `xcode-select --install` as prerequisite.
- [Session name collision on spawn] → daemon returns 409 if `tmux has-session -t` matches; UI surfaces the error.
- [tmux "smallest client wins" sizing can shrink the browser terminal when an iTerm attach coexists] → accepted for P0; document it. Revisit with per-client window sizing later.
- [No auth on loopback] → accepted: bind `127.0.0.1` explicitly and assert it in a test; any 0.0.0.0 bind is a P0 bug.
- [Killing the pty on WS close could orphan ptys if close events are missed] → attach pty lifetime to the socket via a single owner object; add a sweep that kills ptys whose socket readyState is closed.

## Open Questions

- None blocking. (Whether to adopt tmux control mode, and hook-vs-`claude agents --json` status, are explicitly deferred to P1's spike.)
