# Backlog — accepted, not yet scheduled

- **p5 candidate: session reset ("start fresh")** — per-session control that kills the running claude agent and starts a fresh conversation in the SAME tmux session, via `tmux respawn-window -k -t <id> claude` (session entity survives → attached clients stay connected, no WS reconnect). Daemon: `POST /api/sessions/reset {id}` + drop resume metadata so reboot-restore can't resurrect the pre-reset conversation. UI: ↺ control in cell title bar + rail row, two-click confirm ("really?" for ~3s). Old conversation remains reachable via claude's own `--resume` picker. (Requested 2026-07-19, queued behind p4.)

- Folder-trust dialog status blind spot: fresh sessions in never-trusted dirs block invisibly (no hook, agents-json says idle). Candidate: treat a young session with no signals as needs-attention. (Found during P1 e2e.)

- `garage deck <workspace>`: tmux-native tiled layout as terminal-first pit-wall-lite. (Deferred surface decision, IDEA.md.)

- npm publish: package still `private: true`; needs user's npm account + name claim.

- Linux: notifications are a darwin-only no-op; picker is darwin-only (manual path fallback exists).
