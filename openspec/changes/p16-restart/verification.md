# Verification: p16-restart

Date: 2026-09-02. Branch `p16-restart`. Nothing committed.

## Automated

- `npm test`: 126 tests, 126 pass (13 new in `daemon/test/restart.test.js`:
  target planning skip/force/unknown, missing `claudeSessionId`,
  `needsPollBeforePlan`, `waitForPredecessor` return/deadline).
- `cargo test` (wall): 466 pass (23 new: response model parsing, `r` chord
  routing and cancel, `ArmedRestart` arm/fire/expiry/busy→force, workspace
  target selection, notice builders).
- `cargo clippy --all-targets`: two pre-existing warnings, none added.
- `npm run build --workspace ui` clean; `npm run build:tui` produced a
  fresh `wall/dist/garage-wall-darwin-arm64`.

## Live, on a throwaway daemon (`GARAGE_PORT=4748`, scratch `GARAGE_DIR`)

- `POST /api/daemon/restart` → 202 `{pid}`; health answered under the new
  pid within 500ms; the old pid's log shows the 202 then exit; exactly one
  listener on the port throughout (`lsof`).
- TUI attached to :4748 in a scratch tmux pane, keys via `send-keys`:
  - `r` → strip hint `r: r session · a workspace · d daemon`
  - `r d` → `restarting the daemon…`, daemon pid changed, TUI stayed up
    with its tiles
  - `?` → the three new rows after `x x`
  - `r` then `z` → prefix cancelled, no action
- Scratch pane and daemon torn down afterwards.

## Not verified live (by decision)

- `r r`, `r a`, `claude-garage restart --sessions`, and the web `↻`
  control were **not** run against a real session: they respawn the
  user's live Claude Code process, and the user was asked before doing
  so. The tmux call is `respawn-pane -k -c <dir> -t <id> claude --resume
  <claudeSessionId>`, the same operation that successfully repaired three
  live sessions earlier the same day after an agent's smoke test.
- `claude-garage restart` against the user's :4747 daemon (same code path
  as the :4748 test, plus the launcher's pid-change wait).

## Incident

During implementation a subagent smoke-tested `restart --sessions` with
`GARAGE_CLAUDE_CMD=bash` against the shared tmux server and replaced
`claude` in three live sessions with a shell. It repaired them via
`claude --resume` on their session ids; all three are back on claude
2.1.258 with titles intact. Rule recorded: session-mutating verification
is main-session only, with the user's consent per session.
