# Design: p16-restart

## Context

`bin/garage.js` has `fetchHealth`, `daemonIsStale`, `stopStaleDaemon`
(SIGTERM by health pid, lsof fallback, `waitForPortFree`),
`startDetachedDaemon` (spawns `daemon/src/index.js` detached with
`GARAGE_SERVE_UI=1`, log at `$GARAGE_DIR/daemon.log`) and `waitForHealth`.
`daemon/src/sessions.js` restores dead sessions via `createSession(id, dir,
"claude", ["--resume", claudeSessionId])` using metadata from
`registry.js` (`claudeSessionId` learned by the poller). `daemon/src/tmux.js`
wraps tmux calls. The TUI (`wall/`) routes garage-layer keys through
`store.garage_command_for` → `GarageCommand`, has `ArmedAction` for
double-press confirmation (`x x`, `X X`, `X K`), `Notices` for the strip,
and an SSE task with reconnect/backoff that surfaces `AppEvent::Connection`.

## Decisions

**D1 — Session restart is `tmux respawn-pane -k`, not kill + restore.**
`respawn-pane -k -t <id> claude --resume <claudeSessionId>` (with `-c
<dir>`, and the worktree dir for worktree sessions, same resolution as
restore) kills the running `claude` (tmux sends SIGHUP; Claude Code
persists the transcript incrementally, and resume is the same path restore
already trusts) and starts the new command in the same pane. The tmux
session never dies, so the daemon's session identity, the TUI tile, the
dockview panel and the title all survive. Restore's "confirm twice that
resume didn't die" check is reused: if `claude --resume` exits within the
window, respawn again with plain `claude` and report `resumed: false`.
A session with no known `claudeSessionId` restarts as plain `claude` and
is reported that way.

**D2 — Busy sessions are skipped by default.** A restart mid-turn loses
the turn. `working` and `needs-input` are skipped unless `force` (CLI
`--all`, TUI: the armed second press on a busy focused session is the
force — the notice says so before you press again).

**D3 — Daemon self-restart.** `POST /api/daemon/restart`: the daemon
spawns `process.execPath daemon/src/index.js` detached with its own env
plus `GARAGE_PREDECESSOR_PID=<pid>`, replies `202 {pid: <child>}`, then
`app.close()` and `process.exit(0)`. On boot, a daemon with
`GARAGE_PREDECESSOR_PID` set polls `/api/health` until it stops answering
(10s cap, then fail loud) before listening, so the port handoff never
races. Works in both launch modes: TUI mode (daemon already detached) and
web mode (the foreground `claude-garage` process exits; the successor is
detached; the launcher prints that the daemon now runs in the
background). The successor inherits `GARAGE_SERVE_UI` and `GARAGE_PORT`.

**D4 — CLI.** `claude-garage restart [--sessions] [--all]`. Daemon
restart uses the health endpoint when the daemon answers (so the
in-process successor logic is exercised the same way the TUI does it),
else falls back to `stopStaleDaemon` + `startDetachedDaemon`. With
`--sessions`, after the daemon is healthy it calls
`POST /api/sessions/restart {all: true, force: <--all>}` and prints one
line per restarted, skipped and failed session.

**D5 — TUI chords.** `r` opens a one-shot prefix (same mechanism as `X`
→ `X`/`K`): `r r` focused session, `r a` workspace idle/done sessions,
`r d` daemon. The focused-session path uses `ArmedAction` so the first
`r r` arms ("restart <name>? r r again — resumes the conversation";
busy: "…is working — r r again to restart anyway"), the second fires.
`r a` and `r d` fire at once with a notice. The strip shows the result
notice from the API response. `help.rs` gains three rows.

**D6 — Fail loud.** Missing `claudeSessionId` is reported, never
silently defaulted to a fresh session without saying so. tmux errors
propagate as 500 with tmux's stderr. The successor daemon that cannot
take the port within 10s exits non-zero with a message naming the port.

## Risks

- Claude Code receiving SIGHUP mid-write. Restore already resumes after
  hard tmux deaths daily; same guarantee.
- `respawn-pane` requires `remain-on-exit` semantics? No: `-k` kills and
  respawns in one call; the pane never reaches the dead state.
