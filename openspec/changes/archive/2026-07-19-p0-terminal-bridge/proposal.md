# Proposal: p0-terminal-bridge

## Why

claude-garage's entire thesis rests on one architectural claim: a browser UI can be a thin viewer over real tmux-backed Claude Code sessions that survive the tool. P0 proves (or kills) that claim with the smallest possible slice — one daemon, one tmux session, one terminal in the browser — before any UI investment. The competitive research (GAP.md) confirmed no existing tool attaches to real surviving terminals, so this plumbing is the moat; it must be validated first.

## What Changes

- New monorepo scaffold: Node daemon (Fastify + ws) and React + Vite + xterm.js web UI, from an empty project.
- Daemon can spawn a Claude Code session as a detached tmux session named `garage/<workspace>/<label>`, started in the workspace directory.
- Daemon lists garage-owned sessions by parsing `tmux ls` (the `garage/` prefix is the registry — no database).
- WebSocket bridge: `node-pty` running `tmux attach` on the server side, xterm.js rendering on the browser side, bidirectional keystrokes/output.
- Daemon binds `127.0.0.1` only.
- End-to-end survival verification: killing the browser tab (or the daemon) must not kill the session; reattaching restores the live session with scrollback/history.

Out of scope (later changes): workspace grid UI, needs-input status detection, diff panel, reboot restore, npx packaging.

## Capabilities

### New Capabilities
- `session-lifecycle`: spawning, listing, and killing garage-owned tmux sessions; tmux as the sole source of truth; `garage/<workspace>/<label>` naming contract.
- `terminal-bridge`: WebSocket ⇄ pty ⇄ tmux-attach pipeline delivering an interactive terminal in the browser, including reattach semantics and session survival independent of browser/daemon lifetime.

### Modified Capabilities

(none — greenfield change)

## Impact

- New code: `daemon/` (Fastify, ws, node-pty) and `ui/` (React, Vite, xterm.js) packages.
- New runtime dependencies on the user's machine: tmux, Claude Code CLI (`claude`), Node ≥ 20. macOS is the target; Linux should work incidentally; Windows out of scope.
- Security surface: local HTTP/WS server — must bind loopback only; no auth in P0 (loopback trust).
- No existing code or specs affected (first change in the project).
