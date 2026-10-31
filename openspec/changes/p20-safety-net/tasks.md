# Tasks: p20-safety-net

## 1. Isolated tmux (`daemon/src/`)

- [x] 1.1 `tmux.js`: one helper adds `-L $GARAGE_TMUX_SOCKET` to every tmux call when the variable is set
- [x] 1.2 Route the direct tmux calls in `poller.js` and `session-resolve.js` through the helper
- [x] 1.3 Daemon tests set a unique socket per run and kill that tmux server afterwards
- [x] 1.4 Unit test: with the variable set, built argv starts with `-L <socket>`; unset, argv is unchanged

## 2. CI workflow (`.github/workflows/ci.yml`)

- [ ] 2.1 Trigger on `pull_request` and `push` to `main`
- [ ] 2.2 `rust` job on macos-14, ubuntu-latest, windows-latest: `cargo test --lib --bins`, `cargo clippy --all-targets -- -D warnings`; Windows `continue-on-error`
- [ ] 2.3 `daemon` job on macos-14 and ubuntu-latest: install tmux, `npm ci`, `npm test`
- [x] 2.4 `contract` job on macos-14 and ubuntu-latest

## 3. TUI snapshot tests (`wall/`)

- [ ] 3.1 Add `insta` as a dev-dependency
- [ ] 3.2 Snapshots for the rail (family label, blocked family, restorable rows), the strip (tabs, blocked dots) and the tile title (each status)
- [ ] 3.3 CI fails on pending snapshots

## 4. API contract tests (`tests/contract/`)

- [x] 4.1 Runner: start the daemon from `GARAGE_DAEMON_CMD` on a free port, temp `GARAGE_DIR`, unique tmux socket, `GARAGE_CLAUDE_CMD=sh`
- [x] 4.2 One test per route: health, workspaces (list, add, delete), sessions (list, create, delete, restore, restart), hooks and statusline posts, usage, ui visibility, worktree finish
- [x] 4.3 `/api/events`: a session create and a status change each arrive as events
- [x] 4.4 `npm run test:contract` script

## 5. Verification

- [ ] 5.1 A PR on the repo shows the three-system matrix; macOS and Ubuntu green, Windows result recorded
- [ ] 5.2 Running `npm test` locally while the user's daemon runs leaves `tmux ls` on the default server unchanged
- [ ] 5.3 `cargo test --lib --bins`, clippy and `npm run test:contract` green locally
- [ ] 5.4 Record results in `verification.md`
