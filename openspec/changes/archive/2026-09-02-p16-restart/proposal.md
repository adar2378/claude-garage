# Proposal: p16-restart

## Why

Two things go stale under a running wall and today both need hand work:
the garage daemon (after a garage upgrade) and the Claude Code binary
inside every session (after Anthropic ships a release — Claude Code shows
"update available", but a running session keeps the old process until it
is restarted). There is no `restart` command; the README tells you to find
the pid and kill it, and to `/exit` and `claude --resume` each session by
hand. The launcher already knows how to stop a stale daemon and start a
detached one, and the daemon already resumes sessions by Claude session id
for restore. This change exposes both as one word.

## What Changes

- **`claude-garage restart`** restarts the daemon: stop by health pid (lsof
  fallback), start detached, wait for health, print the version. Running
  TUIs and web walls reconnect through the existing SSE reconnect.
- **`claude-garage restart --sessions`** additionally restarts every live
  Claude Code session in place with `tmux respawn-pane -k` running `claude
  --resume <claudeSessionId>` in the same tmux session, so cells, layout
  and titles survive and the conversation continues on the new binary.
  Only `idle` and `done` sessions restart; `working` and `needs-input`
  are skipped and listed. `--all` includes them.
- **Daemon endpoints**: `POST /api/daemon/restart` (self-replace: spawn a
  detached successor, reply 202, close, exit) and
  `POST /api/sessions/restart` with `{id}` or `{all: true, force?: bool}`
  returning `{restarted: [...], skipped: [{id, status}], failed: [...]}`.
- **TUI keys**: `r r` restart the focused session (armed double-press like
  `x x`), `r a` restart every idle/done session in the focused workspace,
  `r d` restart the daemon. All three show a strip notice; `?` lists them.
- **Web wall**: a restart entry in the cell's hover controls, calling the
  session endpoint. Nothing else.

## Capabilities

### New
- `restart`: CLI subcommand, daemon self-restart, in-place session
  restart, TUI chords.

## Out of scope

- Upgrading the garage npm package (stays `npx claude-garage@latest`,
  handled by the stale-daemon gate).
- Restarting a session that has no Claude session id yet (fresh session
  that never wrote a transcript): it restarts as a fresh `claude`, and the
  response says so.
