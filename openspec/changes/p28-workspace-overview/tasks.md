# Tasks: p28-workspace-overview (the `openspec-board` plugin)

Each numbered group ships alone and is testable alone. Group 1 is the only core change and lands first. Groups 2 to 6 are the owner's first priority: a read-only peek pane of one workspace's OpenSpec changes and PR status. Everything from group 2 on is plugin code in `plugins/openspec-board/` and uses only plugin API v1.

## 1. Core change C1: first-party plugins that ship separately (separate commit, lands first)

- [ ] 1.1 `crates/daemon/build.rs`: replace the single `FIXTURE` constant by a list `NOT_EMBEDDED = ["hello", "openspec-board"]`, used by both the embed step and the allowlist step
- [ ] 1.2 Tests in `crates/daemon/src/plugins/`: the allowlist and the payload contain no entry for a listed id and every other first-party folder is still present; `plugins::bundled` does not seed or auto-grant `openspec-board`
- [ ] 1.3 Check `crates/daemon/src/plugins/first_party.rs` and the `plugins/*` workspace glob: a manifest-only or crate-based folder not on the list is still picked up, a listed one is not
- [ ] 1.4 Verify: `cargo test -p garage-daemon` and `cargo test -p garage-daemon --features embed-plugins --lib -- plugins::bundled`

## 2. Plugin skeleton and parsers (`plugins/openspec-board`)

- [ ] 2.1 Folder, `garage-plugin.toml` (permissions `sessions.read`, `ui.widgets`, `ui.panes`, `ui.invoke`, `ui.navigate`; pane, action, widget, events, `[[cli]] tasks`), crate on `garage-plugin-sdk`, `[package.metadata.dist] dist = false`; `plugin schema` validation passes; no dangerous permission
- [ ] 2.2 `tasks.md` parser: checkbox counts, next open task, code fences ignored; tests copied from the `examples/openspec-progress` cases
- [ ] 2.3 State rule (`proposal`/`building`/`complete`) with unit tests for 0/0, 0/n, k/n, n/n
- [ ] 2.4 Verify: `cargo test -p garage-plugin-openspec-board` and `claude-garage plugin link plugins/openspec-board` loads it

## 3. Config and local collector

- [ ] 3.1 Read `config.toml` from `GARAGE_PLUGIN_STATE_DIR` on every run; the plugin never writes it; `cache/` is the only written place (atomic writes, `StateLock` around read-change-write)
- [ ] 3.2 Workspace names and dirs from `GET /api/workspaces`; default discovery: workspace dir plus direct child git repos with `openspec/`; warnings for bad paths, bad TOML and bad regexes
- [ ] 3.3 List changes per repo from the working tree (skip `archive`), read `tasks.md`/`proposal.md`, build rows; mtime cache; no writes in repos
- [ ] 3.4 Tests on temp dirs: no config, empty, bad TOML, bad path, bad regex, scan of children, three changes in each state, a read-only repo directory
- [ ] 3.5 Verify: `cargo test -p garage-plugin-openspec-board config local`

## 4. Peek pane, local changes (owner priority 1)

- [ ] 4.1 `ui` entry: overlay pane drawing the CHANGES table for the focused workspace (state, repo, where, `done/total`, next open task); `j`/`k`, `c`, `r`, `Tab`, `Esc`/`q`; background thread so drawing never blocks
- [ ] 4.2 Starting workspace from the run context's `focus.workspace`, else the first; empty-workspace and warnings rows
- [ ] 4.3 Render-to-buffer snapshot tests: empty workspace, building plus proposal, warnings
- [ ] 4.4 Verify: `cargo test -p garage-plugin-openspec-board pane`, then by hand with `claude-garage plugin link` against a real workspace and `plugin bind openspec-board:board o`

## 5. Remote-branch changes

- [ ] 5.1 `git fetch --prune origin` runner (prompts off, 30 s timeout, failure recorded, 5 min throttle, manual force, only while the pane is open)
- [ ] 5.2 Branch candidate list (`for-each-ref`, glob filter, 30-day default), tree-id skip against the default branch
- [ ] 5.3 Read `tasks.md` and `proposal.md` with `git ls-tree` and `git show` only; cache by tree id
- [ ] 5.4 Merge sources into one row with `where` and a disagreement mark; show it in the pane
- [ ] 5.5 Test with a bare origin and a clone: a branch holds a change the clone lacks; assert it is listed and that `git status --porcelain`, `HEAD` and the files are unchanged
- [ ] 5.6 Test fetch failure (origin removed) still lists already-fetched refs
- [ ] 5.7 Verify: `cargo test -p garage-plugin-openspec-board remote`

## 6. GitHub pull requests (`gh api`)

- [ ] 6.1 `gh` runner: GET only, `-i` for headers, 15 s timeout, ETag cache in `cache/`, `304` handling; first-use line for a workspace with `github = true`
- [ ] 6.2 Rate-limit accounting: backoff below 200, stop at 0 until reset; statuses `gh off`, `gh not logged in` (naming `gh auth login`), `gh offline`, `rate limited until <time>`; documented that `GH_TOKEN` in the user's shell is not visible
- [ ] 6.3 Open and recently merged PRs; link to changes by title, branch, token; task from the title
- [ ] 6.4 Comment scan with configurable `ci` and `review` regexes, newest match, current-vs-stale by head sha prefix; optional native check runs
- [ ] 6.5 Pane: PR rows under each change, check and review marks, status chip in the title
- [ ] 6.6 Tests with a fake `gh` on `PATH`: canned pages, ETags and `304`, rate-limit headers, unauthenticated, timeout; assert no non-GET call
- [ ] 6.7 Verify: `cargo test -p garage-plugin-openspec-board github`

## 7. Waiting on you, sessions, jump and open PR

- [ ] 7.1 Waiting list builder: approve-proposal, owner-review (`owner_needed`), failed-check, needs-input; stale results never raise an item
- [ ] 7.2 Link sessions from `GET /api/sessions` (label, branch token, PR head branch, dir), one change per session; status glyph and count on change rows; waiting block at the top of the pane with the count in the title
- [ ] 7.3 `Enter`: `POST /api/plugins/ui/action` `session.focus` then exit; with no session open the PR. `p`: spawn `open` or `xdg-open` and stay open
- [ ] 7.4 Tests: each waiting kind, stale suppression, merged-PR clearing, unlinked needs-input session, label link, worktree-branch link, first-rule-wins; jump and open-PR against the SDK fake daemon
- [ ] 7.5 Verify: `cargo test -p garage-plugin-openspec-board waiting link jump`

## 8. Status-strip widget

- [ ] 8.1 `tick` entry: read sessions and the last cached model, push the `waiting` widget (`ttl_secs = 300`), amber only when above zero, click opens `openspec-board:board`; always exit 0
- [ ] 8.2 Hooks: `session.status_changed`, `daemon.started`, 60 s timer; optional `background_secs >= 300` refresh from the timer, off by default
- [ ] 8.3 Tests: count with needs-input sessions and cached items, zero state, `daemon.started` redraw, no `git` or `gh` run when `background_secs` is unset
- [ ] 8.4 Verify: `cargo test -p garage-plugin-openspec-board tick`, then by hand: make a session need input and watch the strip

## 9. The `tasks` command

- [ ] 9.1 `cli` entry with `--workspace`, `--json`, `--local`, `--all`, `--refresh`, `--print-config-path`; `needs_daemon = true`, local fallback with a note
- [ ] 9.2 Tests: text block, JSON only on stdout, daemon down, unknown workspace
- [ ] 9.3 Verify: `cargo test -p garage-plugin-openspec-board cli`

## 10. Release pipeline (shipped and updated on its own)

- [ ] 10.1 Manifest `[release]` (`tag = "openspec-board-v{version}"`, `checksums`, four assets) and `[[build]]` fallback; `plugin schema` validation
- [ ] 10.2 CI workflow on tag `openspec-board-v*`: build macOS and Linux (aarch64, x86_64), attach the assets and the checksums file
- [ ] 10.3 Dry run on a prerelease tag; then `plugin install adar2378/claude-garage/plugins/openspec-board` on a clean `GARAGE_DIR` and `plugin update`
- [ ] 10.4 Verify: the install preview lists the permissions and commands and the plugin is not in the `claude-garage` archive

## 11. Docs and privacy

- [ ] 11.1 `plugins/openspec-board/README.md`: install, `config.toml` reference with pattern examples, keys, `tasks`, every command the plugin runs, the `gh auth login` note
- [ ] 11.2 `docs/plugins/README.md`: add `openspec-board` to the examples line, marked opt-in and not bundled
- [ ] 11.3 README Privacy: add "the optional openspec-board plugin runs `git fetch` to your own remotes and, for workspaces that opt in, `gh`; nothing else leaves the machine"
- [ ] 11.4 `LATER.md`: follow-ups (per-repo committed config, label-based approval, notifications, Windows)
- [ ] 11.5 Verify: `npx -y @fission-ai/openspec@1.4.1 validate p28-workspace-overview --strict`, `cargo clippy --workspace --all-targets -- -D warnings`

## 12. Core change C2 (optional, only if the owner answers yes to question 8)

- [ ] 12.1 A generic plugin-owned read route (`PUT` and `GET /api/plugins/{id}/data`, one JSON document per plugin, size limit, existing origin guard) in the daemon, documented as an additive v1 feature with a feature flag
- [ ] 12.2 Contract tests in `tests/contract/` and a `docs/plugins/api.md` entry
- [ ] 12.3 The plugin publishes its model there on each refresh
- [ ] 12.4 Verify: `cargo test -p garage-daemon` and `npm run test:contract`
