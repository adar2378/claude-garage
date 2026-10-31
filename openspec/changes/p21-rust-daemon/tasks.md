# Tasks: p21-rust-daemon

Rules for every task: tmux tests use an isolated socket (`GARAGE_TMUX_SOCKET`), never the default server. `wall` tests run as `cargo test --lib --bins`. No task restarts the user's daemon or touches their sessions. Work on the `feat/p21-rust-daemon` branch in a worktree; nothing merges to `main` before cutover (section 8).

## 1. Scaffold

- [x] 1.1 Root `Cargo.toml` workspace with members `wall`, `crates/protocol`, `crates/daemon`, `crates/cli`; remove `wall/Cargo.toml`'s standalone `[workspace]` table; `cargo build` at the root and inside `wall/` both work; `npm run build:tui` unchanged
- [ ] 1.2 `crates/protocol`: serde types for every request, response and event in the `daemon-api` spec (camelCase on the wire), with round-trip tests against JSON captured from the JS daemon
- [x] 1.3 `crates/daemon`: axum router skeleton, origin guard layer, `{error}` body helper, `GET /api/health`; `SessionBackend` trait; `DaemonEvent` broadcast bus
- [x] 1.4 Module stubs, each with a header naming the JS file it ports: `backend/tmux.rs` (`tmux.js`), `state/registry.rs` (`registry.js`), `state/status.rs` (`status.js`), `poller.rs` (`poller.js`), `session_resolve.rs` (`session-resolve.js`), `api/sessions.rs` (`sessions.js`), `api/workspaces.rs` (`workspaces.js`), `api/events.rs` (`events.js`), `api/hooks.rs` (`hooks.js`), `api/statusline.rs` + `state/context.rs` (`statusline.js`), `transcript.rs` (`transcript.js`), `install.rs` (`settings-install.js`, `claude-home.js`), `api/worktrees.rs` + `git.rs` (`worktrees.js`), `api/editor.rs` (`editor.js`), `api/picker.rs` + `os/` (`picker.js`), `api/visibility.rs` + `os/notify` (`notify.js`), `api/daemon_restart.rs` (`daemon-restart.js`), `security.rs` (`security.js`, `token-auth.js`)
- [ ] 1.5 Test harness: `TestTmux` (unique `-L` socket, `kill-server` on drop), `TestDaemon` (own port, temp `GARAGE_DIR`, `GARAGE_CLAUDE_CMD=sh`, own tmux socket); a test that proves the default tmux server is never contacted
- [x] 1.6 `crates/cli`: `claude-garage` binary with `clap` subcommands `tui`, `daemon`, `restart`, `plugin` (stubbed); `daemon` runs the server in the foreground

## 2. Port the tests first

- [ ] 2.1 Port all 119 JS daemon tests (15 files in `daemon/test/`) to Rust as failing or `#[ignore]`-free pending tests, one Rust test per JS test, names kept recognisable; tmux-touching ones (`sessions.title`, `sessions.context`, `tmux`) use `TestTmux`
- [ ] 2.2 Record the mapping (JS test → Rust test) in `crates/daemon/TESTS.md`

## 3. Port modules (parallel lanes, after 1 and 2)

- [x] 3.1 Lane A, state: `registry` (atomic `state.json` writes, merge rules of `upsertSessionMeta`), `status` (transition rules, `since`, `message` retention), event bus wiring
- [x] 3.2 Lane B, backend: `TmuxBackend` (list, panes, has, create with extended keys and `status off`, respawn, kill, rename, capture, attach command), `normalizeTitle`, `resolveBranch`, poller (2 s tick, `claude agents --json` pid join, cwd fallback, `sessions-changed` diff), `session_resolve`
- [x] 3.3 Lane C, telemetry: hooks route and token check, idle-reminder filter, snippet + install merge with backup; statusline route, context store, rate limits, wrapper script and install with chaining; `transcript` tail reader and context cache; `GET /api/usage`
- [x] 3.4 Lane D, actions: sessions routes (list with live + restorable, create with worktree, delete incl. `?meta=1`, restore, restart with poll-before-plan and resume fallback), workspaces routes (list, put, patch with session rename, delete incl. `?sessions=kill`), worktrees finish, open-editor, pick-directory via `os::pick_directory`, visibility + notifications via `os::notify`, daemon restart with predecessor handoff
  - [x] 3.4 Lane D part: sessions routes (list live + restorable, create with worktree, delete incl. `?meta=1`, restore, restart) and `GET /api/events`; `git.rs` holds `createWorktree`/`currentBranch`/`finishWorktree`
- [x] 3.5 Lane E, launcher: `claude-garage tui` (prerequisite checks for tmux and claude, start-or-reuse daemon detached, version gate restarting a stale daemon by pid, run `garage_wall`), `claude-garage restart [--sessions] [--all]`
- [x] 3.6 All ported tests green: `cargo test -p garage-daemon -p garage-protocol -p claude-garage`

## 4. Parity

- [x] 4.1 Run the p20 contract suite against the JS daemon and the Rust daemon side by side (own ports, `GARAGE_DIR`s and tmux sockets); fix every difference or record it as intended in `verification.md`
- [ ] 4.2 Run the current `garage-wall` binary (built from `main`) against the Rust daemon in an isolated setup and walk the swap-criterion checklist (design.md)

## 5. tmux control mode (after 4)

- [x] 5.1 `tmux -C` reader per backend socket, parsing `%sessions-changed`, `%session-renamed`, `%window-renamed`, `%exit`; publish `SessionsChanged` on the bus immediately
- [x] 5.2 Keep the poll tick as fallback (stays 2 s: tmux sends no notification for pane-title changes, so a slower tick would delay titles; agent status via `claude agents --json` stays on it); reconnect when the control client exits
- [x] 5.3 Contract suite and ported tests still green; measure event latency and idle CPU before and after, record in `verification.md`

## 6. Diff route

- [ ] 6.1 Do not port `GET /api/diff/:workspace`; the Rust daemon returns 404 for it (contract test)
- [ ] 6.2 Delete `daemon/src/diff.js` and its spec at cutover (spec removal is in this change's `workspace-diff` delta)

## 7. Packaging

- [x] 7.1 Add `cargo-dist` 0.33.1 config (`dist-workspace.toml`): macOS arm64/x64, Linux x64/arm64 (gnu), Windows x64; shell, PowerShell, npm and Homebrew installers. One `claude-garage` binary (the wall is linked in, `garage_wall::app`); the first-party plugins and their SHA-256 are embedded at build time (`bundle-plugins` feature, `crates/daemon/build.rs`) because the installers copy only executables. Deviation: dist's npm installer is one `claude-garage` package that downloads the binary from the GitHub release, not per-OS packages. musl skipped (the nested plugin build is not cross-ready)
- [x] 7.2 Generated release workflow (`dist generate`) plus `publish-npm.yml` (OIDC trusted publishing, provenance, `--tag next` for prereleases) and `release-tests.yml` (test gate via `plan-jobs`). The tag must equal the Cargo version (dist plan fails otherwise); release notes come from `CHANGELOG.md`. Prereleases skip npm and Homebrew (dist default)
- [ ] 7.3 (`dist plan` and a host `dist build` pass locally; the tag run and the clean-account run are the user's) Dry run: `dist plan` and a pre-release tag on a fork or a `-rc` version; `npx claude-garage@<rc>` runs the TUI on a clean macOS account or container

## 8. Cutover

- [ ] 8.1 Swap criterion met (design.md), all four checks recorded in `verification.md`
- [ ] 8.2 Ask the user, then switch their wall to the new binary; record their live check
- [ ] 8.3 Delete `daemon/` and `bin/garage.js` (kept until 8.1/8.2: they are the parity oracle for `npm test` and the contract tests). Done early: the Node release workflow and the `build:tui` / `build:plugins` scripts are gone, and the root `package.json` is `private` with no `bin`/`files`, so nothing Node can be published; the published package is cargo-dist's
- [ ] 8.4 `CHANGELOG.md` entry; release as the next minor version
- [ ] 8.5 `openspec validate p21-rust-daemon --strict`; archive
