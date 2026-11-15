# Tasks: p28-workspace-overview (the `openspec-board` plugin)

Each numbered group ships alone and is testable alone. There is no core change. Everything is plugin code in `extras/openspec-board/` (own `[workspace]`, own version, own `Cargo.lock`) and uses only plugin API v1. Groups 1 to 3 are the owner's first priority: a read-only peek pane of one workspace's OpenSpec changes and PR status. Each group declares in the manifest only what it implements, and every group's tests build their own temp fixture (temp dirs and repos, a bare origin and a clone, a fake `gh`, the SDK fake daemon), never a real workspace or an earlier group's output. Test commands take one filter each (`cargo test` accepts a single positional filter), or `-- --exact <name>`.

## 1. Peek pane, local changes (owner priority 1)

- [x] 1.1 Folder `extras/openspec-board/`: `Cargo.toml` (own `[workspace]`, `version = "0.1.0"`, SDK path dependency like `examples/openspec-progress`), `Cargo.lock`, `garage-plugin.toml` with `permissions = ["ui.panes"]`, `[[build]]` (`cargo build --release`), the `board` pane and the `board` action (`default_key = "o"`), commands pointing at `{plugin_dir}/target/release/openspec-board`; no `{bin}`, no `[release]`, no widget, no event, no CLI, no other permission yet
- [x] 1.2 `tasks.md` parser (checkbox counts, next open task, code fences ignored; cases copied from `examples/openspec-progress`) and the state rule (`proposal`/`building`/`complete`; 0/0, 0/n, k/n, n/n)
- [x] 1.3 Config: read `config.toml` from `GARAGE_PLUGIN_STATE_DIR` on every run, never write it; `cache/` (mode 0700, files 0600) is the only written place (atomic writes, `StateLock`); workspace names and dirs from `GET /api/workspaces`; default discovery (workspace dir plus direct child git repos with `openspec/`); warnings for bad paths, bad TOML and bad regexes
- [x] 1.4 Local collector: list changes per repo from the working tree (skip `archive`, skip names found in the default branch's `archive/`), read `tasks.md`/`proposal.md`, build rows; mtime cache; no writes in repos
- [x] 1.5 `ui` entry: overlay pane drawing the CHANGES table for the focused workspace (state, repo, where, `done/total`, next open task); `j`/`k`, `c`, `r`, `Tab`, `Esc`/`q`; background thread so drawing never blocks; starting workspace from `focus.workspace`, else the first; empty-workspace and warnings rows
- [x] 1.6 Tests on temp fixtures: no config, empty, bad TOML, bad path, bad regex, scan of children, three changes in each state, a read-only repo directory, an archived name skipped; render-to-buffer snapshots (empty workspace, building plus proposal, warnings)
- [x] 1.7 Verify, one command per area: `cargo test --manifest-path extras/openspec-board/Cargo.toml parser::`, `cargo test --manifest-path extras/openspec-board/Cargo.toml config::`, `cargo test --manifest-path extras/openspec-board/Cargo.toml local::`, `cargo test --manifest-path extras/openspec-board/Cargo.toml pane::`; `cargo test --manifest-path extras/openspec-board/Cargo.toml manifest_tests` passes (`plugin schema` takes no argument, so the manifest is checked by a test and by `plugin link`); by hand `cargo build --release --manifest-path extras/openspec-board/Cargo.toml` (`plugin link` does not build; `plugin dev` does), then `claude-garage plugin link extras/openspec-board` and `plugin bind openspec-board:board o`

## 2. Remote-branch changes

- [ ] 2.1 Fetch runner: `git fetch --prune --no-auto-gc --no-auto-maintenance --no-write-fetch-head origin`, stdin null, `GIT_TERMINAL_PROMPT=0`, `GIT_SSH_COMMAND='ssh -o BatchMode=yes'`, 30 s timeout, failure recorded, 5 min throttle, manual force with `r`; runs only while the pane is open (or on `r`, or an explicit `tasks --refresh`); no fetch from any other consumer
- [ ] 2.2 Branch candidate list (`for-each-ref`, glob filter, 30-day default), tree-id skip against the default branch (`symbolic-ref`, then `main`, `master`)
- [ ] 2.3 Read `tasks.md` and `proposal.md` with `git ls-tree` and `git show` only; cache by tree id
- [ ] 2.4 Merge sources into one row with `where` and a disagreement mark; show it in the pane
- [ ] 2.5 Tests with a bare origin and a clone made in the test: a branch holds a change the clone lacks; assert it is listed and that `git status --porcelain`, `HEAD`, the files and every ref outside `refs/remotes` are unchanged and no `FETCH_HEAD` exists; fetch failure (origin removed) still lists already-fetched refs; an archived name on the default branch is skipped; assert the fetch command line and environment
- [ ] 2.6 Verify, one command per area: `cargo test --manifest-path extras/openspec-board/Cargo.toml remote::`, `cargo test --manifest-path extras/openspec-board/Cargo.toml fetch::`

## 3. GitHub pull requests (`gh api`)

- [ ] 3.1 `gh` runner: GET only, `-i`, 15 s timeout, ETag cache in `cache/`; parse the **status line** (`200`, `304`, `401`, `403`, ...) and never the exit code, because `gh api` exits 1 on HTTP 304; treat a missing status line as "never reached GitHub"; parse headers on every response; first-use line for a workspace with `github = true`; `github` is off by default
- [ ] 3.2 Rate-limit accounting from `X-Ratelimit-Remaining` and `X-Ratelimit-Reset` (also on a 304): backoff below 200, stop at 0 until reset; statuses `gh off`, `gh not logged in` (naming `gh auth login`), `gh offline`, `rate limited until <time>`
- [ ] 3.3 Open and recently merged PRs; per-PR changed files (re-read when the head moves); link a PR to a change when the change name appears anywhere in the head branch, title or diff paths (diff path wins, then longest name); task number from the title pattern
- [ ] 3.4 Comment scan with configurable `ci` and `review` regexes, newest match, current-vs-stale by head sha prefix; `owner_needed` matched against the whole body of the newest review comment; optional native check runs
- [ ] 3.5 Pane: PR rows under each change, check and review marks, status chip in the title
- [ ] 3.6 Tests with a fake `gh` on `PATH`: canned pages, ETags, `304` served with exit status 1, rate-limit headers, unauthenticated, timeout; branch `claude/anything` and title `docs(openspec): propose <change>` link; a short name does not beat a longer matching one; `Owner approval: yes` on its own line is found; assert no non-GET call and no call at all when `github` is off
- [ ] 3.7 Verify, one command per area: `cargo test --manifest-path extras/openspec-board/Cargo.toml github::`, `cargo test --manifest-path extras/openspec-board/Cargo.toml linking::`

## 4. Waiting on you, sessions, jump and open PR

- [ ] 4.1 Manifest: add `sessions.read`, `ui.invoke`, `ui.navigate` (nothing earlier)
- [ ] 4.2 Waiting list builder: approve-proposal (an open PR whose diff adds the change folder; none at all with GitHub off; cleared by a merged PR or an archived name), owner-review (`owner_needed`), failed-check, needs-input; stale results never raise an item
- [ ] 4.3 Link sessions from `GET /api/sessions` (label, branch, PR head branch, dir), one change per session; garage sessions only; status glyph and count on change rows; waiting block at the top of the pane with the count in the title
- [ ] 4.4 `Enter`: `POST /api/plugins/ui/action` `session.focus` then exit; with no session open the PR. `p`: spawn `open` or `xdg-open` and stay open
- [ ] 4.5 Tests: each waiting kind, GitHub off gives no approve-proposal item, stale suppression, merged-PR clearing, unlinked needs-input session, label link, worktree-branch link, first-rule-wins; jump and open-PR against the SDK fake daemon
- [ ] 4.6 Verify, one command per area: `cargo test --manifest-path extras/openspec-board/Cargo.toml waiting::`, `cargo test --manifest-path extras/openspec-board/Cargo.toml sessions::`, `cargo test --manifest-path extras/openspec-board/Cargo.toml jump::`

## 5. Status-strip widget

- [ ] 5.1 Manifest: add `ui.widgets`, `requires = ["slots:strip.right"]`, the `waiting` widget, and the hooks `session.status_changed`, `daemon.started` and a 60 s `timer`, all running `tick` (nothing earlier)
- [ ] 5.2 `tick`: read `GET /api/sessions` and the last cached model only (no `git`, no `gh`, no repo files); push the widget with `ttl_secs = 300`; `needs_input` token only while a session waits on input, `warn` for proposals and failed CI, dim at zero; `~` when the cached model is older than 15 minutes (no tooltip: the age is in the pane title); click opens `openspec-board:board`; always exit 0
- [ ] 5.3 The pane writes the model to `cache/` after each refresh (atomic, `StateLock`)
- [ ] 5.4 Tests: count with needs-input sessions and cached items, colour choice for each case, stale mark, zero state, `daemon.started` redraw, no `git` or `gh` process and no repo read during a tick
- [ ] 5.5 Verify: `cargo test --manifest-path extras/openspec-board/Cargo.toml tick::`, then by hand: make a session need input and watch the strip

## 6. The `tasks` command

- [ ] 6.1 Manifest: add `[[cli]] name = "tasks"` with `needs_daemon = false` (nothing earlier)
- [ ] 6.2 `cli` entry with `--workspace`, `--json`, `--local` (skip fetch and GitHub), `--all`, `--refresh`, `--print-config-path`; call the daemon API with a short timeout and use it only if it answers, else local data with a note; no fetch and no `gh` without `--refresh`
- [ ] 6.3 Tests: text block, JSON only on stdout, daemon down (no daemon needed, exit 0), unknown workspace, no network use without `--refresh`
- [ ] 6.4 Verify: `cargo test --manifest-path extras/openspec-board/Cargo.toml cli::`

## 7. Build, docs, privacy and CI

- [ ] 7.1 Check a fresh `claude-garage plugin install adar2378/claude-garage/extras/openspec-board` on a clean `GARAGE_DIR` builds (the SDK path dependency resolves); if not, depend on the SDK by git revision; check `plugin update` rebuilds
- [ ] 7.2 `extras/openspec-board/README.md`: install, `config.toml` reference with examples, keys, `tasks`, every command the plugin runs, the `gh auth login` note, that config is lost on `plugin remove --purge`, that **`cache/` holds private PR titles and comment bodies**, that fetch talks only to the user's own remote and writes nothing outside `refs/remotes` besides objects, that cloud sessions are not shown
- [ ] 7.3 `docs/plugins/README.md`: add `openspec-board` to the examples line, marked opt-in, build from source, not bundled
- [ ] 7.4 README Privacy: add "the optional openspec-board plugin runs `git fetch` to your own remote and, for workspaces that opt in, `gh`; its cache holds private PR text on disk; nothing else leaves the machine"
- [ ] 7.5 CI job for the crate (fmt, clippy, test on changes under `extras/openspec-board/`), **no tag trigger**; confirm no plugin tag is created and `scripts/check-release-tag.sh` is untouched
- [ ] 7.6 `LATER.md`: a release path for the plugin (a tag scheme garage's cargo-dist ignores, assets, checksums, `{bin}`), per-repo committed config, label-based approval, notifications, Windows
- [ ] 7.7 Verify: `npx -y @fission-ai/openspec@1.4.1 validate p28-workspace-overview --strict`, `cargo clippy --manifest-path extras/openspec-board/Cargo.toml --all-targets -- -D warnings`
