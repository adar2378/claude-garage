# Tasks: p16-restart

## 1. Daemon (`daemon/src/`)

- [x] 1.1 `tmux.js`: `respawnPane(id, dir, command, extraArgs)` wrapping `tmux respawn-pane -k -c <dir> -t <id> <command> <args…>`; error carries tmux stderr
- [x] 1.2 `sessions.js`: `POST /api/sessions/restart` `{id} | {all, force}` → restarted/skipped/failed, reusing restore's dir resolution (worktree) and died-on-resume confirmation; plain `claude` + `resumed: false` when no `claudeSessionId`
- [x] 1.3 New `daemon/src/daemon-restart.js`: `POST /api/daemon/restart` (spawn successor with `GARAGE_PREDECESSOR_PID`, 202, close, exit); boot-time wait-for-predecessor in `index.js`, 10s fail-loud
- [x] 1.4 Tests: restart route selection logic (skip/force, missing session id), successor wait logic with a fake health probe

## 2. Launcher (`bin/garage.js`)

- [x] 2.1 `restart` subcommand: daemon via endpoint when healthy else stop+start; wait for health; print version and pid
- [x] 2.2 `--sessions` / `--all`: call the session endpoint, print per-session lines
- [x] 2.3 Usage line for unknown subcommands (`tui`, `restart`)

## 3. TUI (`wall/src/`)

- [x] 3.1 `api/client.rs`: `restart_session(id, force)`, `restart_sessions(workspace, force)`, `restart_daemon()`; models for the response
- [x] 3.2 `store.rs`: `GarageCommand::RestartFocused | RestartWorkspace | RestartDaemon`; `r` prefix chord like `X`; `ArmedAction` for `r r` with the busy warning; unit tests for chord routing and arming
- [x] 3.3 `runtime.rs`: dispatch the three commands, notices from the responses
- [x] 3.4 `help.rs`: three rows; `BINDINGS` length updated

## 4. Web wall (`ui/src/`)

- [x] 4.1 `lib/api.js` `restartSession(id, force)`; restart control in `SessionCellTab` hover cluster with the existing armed-close pattern

## 5. Docs + verification

- [x] 5.1 README: "Restarting" section under Web wall/TUI; keybindings table rows; CHANGELOG
- [x] 5.2 `npm test`, `cargo test`, `cargo clippy`, `npm run build --workspace ui` green
- [~] 5.3 (partial: daemon + TUI chords on a throwaway daemon; real-session respawn deferred to the user, see verification.md) Live: `claude-garage restart` with the user's TUI attached (box → live); `r r` on an idle session after confirming with the user; skip of a working session; daemon `r d`; record in `verification.md`
