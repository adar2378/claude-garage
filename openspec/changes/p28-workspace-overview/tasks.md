# Tasks: p28-workspace-overview

Each numbered group ships alone and is testable alone. Groups 1 to 7 are the owner's first priority: a read-only peek at one workspace's OpenSpec changes and PR status.

## 1. Crate and parsers (`crates/overview`)

- [ ] 1.1 New library crate `garage-overview` (no UI, no tokio): types for change, task progress, PR, check, review, waiting item
- [ ] 1.2 `tasks.md` parser: checkbox counts, next open task, code fences ignored; tests copied from the `examples/openspec-progress` cases
- [ ] 1.3 State rule (`proposal`/`building`/`complete`) with unit tests for 0/0, 0/n, k/n, n/n
- [ ] 1.4 Verify: `cargo test -p garage-overview`

## 2. Declaring repos (`overview.toml`)

- [ ] 2.1 Parse `~/.garage/overview.toml` (honour `GARAGE_DIR`), `~` expansion, defaults, warnings for bad paths and regexes
- [ ] 2.2 Default discovery: workspace dir plus direct child git repos with `openspec/`
- [ ] 2.3 Tests on temp dirs: no file, empty file, bad TOML, bad path, bad regex, scan of children
- [ ] 2.4 Verify: `cargo test -p garage-overview config`

## 3. Local changes collector

- [ ] 3.1 List changes per repo from the working tree (skip `archive`), read `tasks.md`, build rows
- [ ] 3.2 mtime cache so unchanged files are not re-read; no writes anywhere
- [ ] 3.3 Test: a temp repo with three changes in each state yields the expected rows; a read-only directory still works
- [ ] 3.4 Verify: `cargo test -p garage-overview local`

## 4. Remote-branch changes

- [ ] 4.1 `git fetch --prune origin` runner (prompts off, 30 s timeout, failure recorded, 5 min throttle, manual force)
- [ ] 4.2 Branch candidate list (`for-each-ref`, glob filter, 30-day default), tree-id skip against the default branch
- [ ] 4.3 Read `tasks.md` and `proposal.md` with `git ls-tree` and `git show` only; cache by tree id
- [ ] 4.4 Merge sources into one row with `where` and a disagreement mark
- [ ] 4.5 Test with a bare origin and a clone: a branch holds a change the clone lacks; assert it is listed and that `git status --porcelain`, `HEAD` and the files are unchanged
- [ ] 4.6 Test fetch failure (origin removed) still lists already-fetched refs
- [ ] 4.7 Verify: `cargo test -p garage-overview remote`

## 5. GitHub pull requests (`gh api`)

- [ ] 5.1 `gh` runner: GET only, `-i` for headers, 15 s timeout, ETag cache under `~/.garage/cache/overview/`, `304` handling
- [ ] 5.2 Rate-limit accounting: backoff below 200, stop at 0 until reset; statuses `off`, `unauth`, `offline`, `ratelimited`
- [ ] 5.3 Open and recently merged PRs; link to changes by title, branch, token; task from the title
- [ ] 5.4 Comment scan with configurable `ci` and `review` regexes, newest match, current-vs-stale by head sha prefix; optional native check runs
- [ ] 5.5 Tests with a fake `gh` on `PATH`: canned pages, ETags and `304`, rate-limit headers, unauthenticated, timeout; assert no non-GET call
- [ ] 5.6 Verify: `cargo test -p garage-overview github`

## 6. Daemon endpoint (needed by the overlay)

- [ ] 6.1 `crates/daemon/src/api/overview.rs`: `GET /api/overview[?workspace]`, answers from cache, never blocks on `git`/`gh`
- [ ] 6.2 Consumer tracking and the refresh loop (5 s local, 60 s GitHub, 5 min fetch); idle means nothing runs
- [ ] 6.3 `POST /api/overview/refresh` and `event: overview`
- [ ] 6.4 Contract tests in `tests/contract/`: route shape, 404, offline GitHub, origin guard on POST, event after refresh
- [ ] 6.5 Verify: `cargo test -p garage-daemon` and `npm run test:contract`

## 7. Peek view (owner priority 1)

- [ ] 7.1 `claude-garage tasks` in `crates/cli` with `--workspace`, `--json`, `--local`, `--all`; daemon first, in-process local fallback; reserve the word in the plugin CLI checker and `docs/plugins/cli.md`
- [ ] 7.2 Wall overlay `wall/src/ui/overview.rs` (changes, tasks, next open task, PRs with CI and review, current/stale marks); opens on the chosen key, `j`/`k`, `r`, `Esc`, `Tab`, `c`
- [ ] 7.3 Confirm the key is free in `keymap.rs` and `help.rs`; add it to the `?` help
- [ ] 7.4 insta snapshots: empty workspace, building plus proposal, PR with current and stale verdicts, offline chip
- [ ] 7.5 Verify: `cargo test -p garage-wall --lib --bins`, `cargo test -p claude-garage`, then by hand against a real workspace

## 8. Waiting on you

- [ ] 8.1 Waiting list builder: approve-proposal, owner-review (`owner_needed`), failed-check, needs-input; stale results never raise an item
- [ ] 8.2 Show the list at the top of the overlay and in `tasks` output; count in the overlay title
- [ ] 8.3 Unit tests for each kind, stale suppression, merged-PR clearing, unlinked needs-input session
- [ ] 8.4 Verify: `cargo test -p garage-overview waiting`

## 9. Sessions linked to changes

- [ ] 9.1 Link rules (label, branch token, PR head branch) over the daemon's session list, one change per session
- [ ] 9.2 Session status glyph and count on change rows
- [ ] 9.3 `Enter` in the overlay focuses the session through the existing jump action (workspace and view); `p` opens the PR
- [ ] 9.4 Tests: label link, worktree-branch link, first-rule-wins, no link; overlay jump test with a fake wall state
- [ ] 9.5 Verify: `cargo test -p garage-overview link` and `cargo test -p garage-wall --lib`

## 10. Rail hint (optional, small)

- [ ] 10.1 A waiting count next to the workspace in the rail, only when above zero and amber only for human-needed items
- [ ] 10.2 Snapshot test; drop the task if the rail is too busy

## 11. Site and web readers

- [ ] 11.1 Document the endpoint and model in `docs/` and the plugin API doc (additive fields only)
- [ ] 11.2 Confirm the CORS/origin behaviour matches `GARAGE_UI_ORIGINS` for `GET`; note what the site must set
- [ ] 11.3 Verify: contract test reads the endpoint with and without an `Origin` header

## 12. Refresh polish

- [ ] 12.1 `r` in the overlay and `claude-garage tasks --refresh` both call the refresh route
- [ ] 12.2 Status chip text: `gh off`, `gh not logged in`, `gh offline 9m`, `rate limited until 14:05`, `fetch failed 12m ago`
- [ ] 12.3 First-use line for a workspace with `github = true` explaining that `gh api` runs against github.com
- [ ] 12.4 Verify: unit tests for chip text; manual run with the network off

## 13. Docs and privacy

- [ ] 13.1 `docs/overview.md`: `overview.toml` reference with the pattern examples, key, CLI, API model
- [ ] 13.2 README: add the key to the Keys table and extend Privacy ("`git fetch` to your own remotes, and `gh` for workspaces that opt in; nothing else leaves the machine")
- [ ] 13.3 `LATER.md`: note follow-ups (per-repo committed config, label-based approval, notifications)
- [ ] 13.4 Verify: `npx -y @fission-ai/openspec@1.4.1 validate p28-workspace-overview --strict`, `cargo clippy --workspace --all-targets -- -D warnings`
