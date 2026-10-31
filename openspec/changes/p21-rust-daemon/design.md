## Context

The daemon (`daemon/src/*.js`, 3,189 lines, 23 files) is a Fastify server on `127.0.0.1:4747`. It owns no sessions: tmux does. It reads and writes `~/.garage/state.json` (workspaces and resume metadata), keeps an in-memory status store fed by a 2-second poller (`claude agents --json` joined to `tmux list-panes` by pid) and by Claude Code HTTP hooks, and streams changes to the TUI over Server-Sent Events. `bin/garage.js` checks prerequisites, starts the daemon detached, restarts a stale one (version gate on `/api/health`), finds or builds the TUI binary and runs it.

The TUI (`wall/`) is a standalone Cargo workspace (its `Cargo.toml` has an empty `[workspace]` table) with a `garage_wall` library and a `garage-wall` binary. It talks to the daemon over HTTP with `ureq` and parses JSON by hand (`serde_json::Value`).

The user runs garage every day. Their v0.6.2 daemon and their live tmux sessions must not be touched while this change is built.

## Goals / Non-Goals

**Goals:**
- One Rust daemon with the same HTTP contract as the JS one, proven by tests that run against both.
- Seams the later phases need: `SessionBackend` trait, OS module, event bus, shared protocol types.
- One `claude-garage` binary, shipped per platform, with no runtime to install.
- tmux isolation so every automated test is safe on the user's machine.

**Non-Goals:**
- Windows support (p26 adds a backend behind the trait).
- Plugin host (p24; the `plugin` subcommand is only reserved here).
- TUI refactors (p22) or new TUI behaviour.
- Changing `state.json`, hook payloads, or the statusline wrapper format.

## Decisions

**1. Workspace layout.** Root `Cargo.toml` with members `wall`, `crates/protocol`, `crates/daemon`, `crates/cli`. `wall/` stays where it is, because `npm run build:tui`, `bin/garage.js` and `release.yml` hardcode its path until cutover. Its standalone `[workspace]` table is removed so it joins the root workspace; `cargo build` inside `wall/` keeps working. Alternative: put the binary in `crates/daemon`. Rejected: the binary also runs the TUI, so it belongs in neither library.

**2. Crates.**

| Job | Crate |
|---|---|
| Runtime | `tokio` (already used by `wall`) |
| HTTP + SSE | `axum`, `tower-http` |
| CLI | `clap` (derive) |
| Data and config | `serde`, `serde_json`, `toml`, `schemars` |
| Per-OS paths | `directories` (default `~/.garage` kept as-is for compatibility) |
| Notifications | `notify-rust` |
| Folder picker | `rfd` |
| Clipboard | `arboard` |
| File watching | `notify` |
| Local IPC (reserved for plugins) | `interprocess` |
| Process info | `sysinfo` |
| Logs | `tracing`, `tracing-subscriber` |
| Tests | `tokio::test`, `axum-test` or `reqwest` against a bound port, `insta` for JSON snapshots |

Git stays a subprocess (`git -C <dir> ...`), as today. A library reimplementation of worktree add/remove and merge is riskier than git's own CLI.

**3. `crates/protocol`.** `serde` types for every request, response and event in the `daemon-api` spec: `Session`, `SessionStatus` (`idle | working | needs-input | done | restorable`), `Context`, `Worktree`, `Workspace`, `RateLimits`, `Health`, `Event`. `crates/daemon` serializes them; `wall` deserializes them, replacing hand-parsed `Value`s over time (p22 can finish that). Field names stay camelCase on the wire (`#[serde(rename_all = "camelCase")]`).

**4. `SessionBackend` trait** (in `crates/daemon/src/backend/`):

```rust
#[async_trait]
pub trait SessionBackend: Send + Sync {
    async fn list(&self) -> Result<Vec<BackendSession>>;          // id, dir, attached, created_at
    async fn panes(&self) -> Result<HashMap<String, PaneInfo>>;   // cwd, title, pid
    async fn has(&self, id: &str) -> Result<bool>;
    async fn create(&self, id: &str, dir: &Path, cmd: &str, args: &[String]) -> Result<()>;
    async fn respawn(&self, id: &str, dir: &Path, cmd: &str, args: &[String]) -> Result<()>;
    async fn kill(&self, id: &str) -> Result<()>;
    async fn rename(&self, from: &str, to: &str) -> Result<()>;
    async fn capture(&self, id: &str, lines: usize) -> Result<String>;
    fn attach_command(&self, id: &str) -> Vec<String>;             // what the TUI spawns in a PTY
}
```

`TmuxBackend` implements it by porting `daemon/src/tmux.js` and the two direct `tmux` calls in `poller.js` and `session-resolve.js`. It always passes `-L <socket>` when `GARAGE_TMUX_SOCKET` is set. Unset means the default server, which is the user's normal setup. `ensureExtendedKeys` runs inside `create` and at startup, as today.

**5. tmux isolation, from the first commit.** Every daemon test that touches tmux creates a `TmuxBackend` with a unique socket (`garage-test-<pid>-<n>`) and kills that server (`tmux -L <socket> kill-server`) on drop. No test, agent or CI job uses the default tmux server. The TUI's attach path (`wall/src/pty.rs`) gets the same socket through `attach_command`, so an end-to-end test can run a whole isolated garage.

**6. Event bus.** One `tokio::sync::broadcast` channel of `DaemonEvent` (`StatusTransition { id, from, to, since }`, `SessionsChanged`). The status store, poller, restart route and (later) control-mode reader publish. SSE clients, notifications and (later) plugins subscribe. This replaces `statusEvents` and `pollerEvents`.

**7. Port first, improve second.** The first version replicates current behaviour exactly, including the 2-second poll and the `claude agents --json` pid join, so ported tests and contract tests prove parity. Only after parity does a separate task switch to tmux control mode: one long-lived `tmux -C` client per backend socket, parsing `%sessions-changed`, `%session-renamed`, `%window-renamed` and `%exit` notifications, with a slow poll kept as a fallback for agent status (Claude Code status still comes from `claude agents --json` and hooks). Mixing the two would make a port bug look like a redesign bug.

**8. OS module** (`crates/daemon/src/os/`): `notify(title, body)`, `pick_directory() -> Option<PathBuf>`, `open_window(cmd)`, `clipboard_text()`. One file per OS behind `cfg`, each returning `Unsupported` where an OS lacks the feature. The `POST /api/pick-directory` route maps `Unsupported` to 501, as today on non-macOS. Notifications keep today's rule: fire on a transition to `needs-input` only when no UI client reported itself visible within 45 seconds.

**9. One binary.** `crates/cli` builds `claude-garage` with subcommands:
- `tui` (default with no subcommand): prerequisite checks, start or reuse the daemon, version gate, run `garage_wall::run`.
- `daemon`: run the server in the foreground (the detached start re-executes the same binary with `daemon`).
- `restart [--sessions] [--all]`: same behaviour as today's launcher.
- `plugin`: reserved, prints "coming in a later release".

The daemon's self-restart (`POST /api/daemon/restart`) re-executes `std::env::current_exe()` with `daemon` and `GARAGE_PREDECESSOR_PID`, keeping the port handoff from p16.

**10. Distribution: `cargo-dist`.** It builds the binary per target (macOS arm64 and x64, Linux x64 and arm64; Windows added in p26) and generates an npm package with per-OS binary packages, a Homebrew formula and a shell installer. `npx claude-garage` keeps working. The release workflow from v0.6.2 is replaced by cargo-dist's generated one, keeping npm trusted publishing. Until cutover, the current npm package and `release.yml` are unchanged.

**11. Diff route removed.** `GET /api/diff/:workspace?sessionId=` returned `{ files: [{ path, oldPath, changeType, additions, deletions, binary, diff, truncated }], truncated, root, branch }`. The TUI has not called it since 0.5.0. Porting 465 lines for no caller is waste; the archived `workspace-diff` spec keeps the contract if review comes back (likely as a plugin).

**12. Configuration.** Environment variables keep their names and meaning: `GARAGE_PORT`, `GARAGE_DIR`, `GARAGE_CLAUDE_CMD`, `GARAGE_CLAUDE_HOME`, `GARAGE_EDITOR_CMD`, `GARAGE_UI_ORIGINS`, `GARAGE_PREDECESSOR_PID`, `GARAGE_TUI_PORT`. New: `GARAGE_TMUX_SOCKET`. A `config.toml` is out of scope here (p23 and p24 add one).

## Swap criterion

The Rust daemon replaces the JS daemon only when **all** of these hold, recorded in `verification.md`:

1. **Ported tests green:** every one of the 119 JS daemon tests (15 files) has a Rust equivalent, and `cargo test` for `crates/daemon` passes with tmux isolation.
2. **Contract parity:** the contract suite from p20-safety-net passes against the JS daemon and the Rust daemon, each running on its own port, `GARAGE_DIR` and tmux socket.
3. **Unchanged TUI:** the current `garage-wall` binary, built from `main` before this change, runs against the Rust daemon in an isolated setup (own port, `GARAGE_DIR`, tmux socket): create, attach, restart, restore, kill, rename and delete a workspace, hook and statusline posts, usage bar data.
4. **User live check:** the user switches their own wall to the new binary and confirms it, after being asked. Before that point, nothing in this change restarts the user's daemon or touches their tmux sessions.

If any check fails, the JS daemon stays the shipped one, and the Rust daemon stays behind `claude-garage daemon` for testing only.

## Risks / Trade-offs

- **Rewriting code the user depends on daily.** Mitigated by the swap criterion, by keeping the JS daemon shipped until it passes, and by contract tests running against both.
- **Subtle behaviour in the JS code** (status merge rules in `status.js`, `applyAgentStatus` never downgrading `needs-input`/`done` to idle, resume-then-fallback timing in `spawnClaudeResumed`, the poll-before-plan rule in restart). The ported tests cover them; the `daemon-api` spec names them.
- **cargo-dist changes the release path** that just started working (trusted publishing). Kept as the last task, behind its own verification run.
- **Windows** is not proven by this change. The trait is shaped by today's tmux needs plus the host-process design in the open Windows PR; p26 may need to widen it.
- **Fastify-specific behaviour** (`forceCloseConnections`, error body format for unmatched routes and bad JSON) must be matched where the TUI depends on it. Contract tests decide.

## Open Questions

- Should `npx claude-garage` keep working through the npm per-OS packages, or should Homebrew become the main install path? The design keeps npm.
- Should the Rust daemon keep reading `~/.garage` on Linux, or move to the XDG directories from `directories`? The design keeps `~/.garage` everywhere for now.

## Resolved (2026-10-31, user: all defaults)

- `npx claude-garage` stays the main install path; Homebrew is added via cargo-dist.
- `~/.garage` stays the data folder on every OS; `GARAGE_DIR` overrides it.
- The diff route is removed (404); the TUI has not used it since 0.5.0.
- A fourth crate, `crates/cli`, holds the single `claude-garage` binary.
