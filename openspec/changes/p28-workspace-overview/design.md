# Design: p28-workspace-overview (the `openspec-board` plugin)

## Mockup (the overlay pane, workspace `kowboy` focused)

```
┌ openspec-board · kowboy ─────────────────────── 3 waiting on you · fetched 2m ago · gh ok ┐
│ WAITING ON YOU                                                                            │
│ ▸ ● proposal  billing-v2          approve proposal            api            no PR yet    │
│   ◆ PR api#212  [billing-v2 2.1]  review: owner approval needed   for head a1b2c3d        │
│   ✗ PR web#88   [search-ui 1.3]   CI (full): FAIL e4f5a6b (head)                          │
│   ● session  billing-v2-ui        needs input: "Allow Bash(npm test)?"        4m          │
│                                                                                           │
│ CHANGES                                    tasks             next open       PRs  sessions │
│   billing-v2     proposal    api  origin/feat/billing-v2   0/14  1.1 schema      –   ◐ 1  │
│   search-ui      building    web  local                    6/11  2.1 debounce  2 ● ◆   ◐ 2 │
│   ├ [search-ui 1.3] web#88  CI FAIL e4f5a6b (head) · review n/a                            │
│   └ [search-ui 2.1] web#91  CI PASS 9c8d7e6 (head) · review APPROVED 9c8d7e6 (head)        │
│   audit-log      building    api  local + origin/feat/audit   9/9  –  complete     1 ✓   ○ 0 │
│   └ [audit-log 3.2] api#209 CI PASS 77aa11c · review PASS 5b6c7d8 (STALE: head is 77aa11c) │
│                                                                                           │
│ j/k move  Enter jump to session  p open PR  r refresh  c change detail  Tab workspace  Esc│
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

The status strip (right side) while the pane is closed: `board 3` (amber while any item needs the human, dim `board 0` otherwise); clicking it opens the pane.

Legend: `●` needs the human now, `◆` PR waiting, `✗` failed check, `◐` working session, `✓` all checks passed. "head" means the verdict or check names the PR's current head commit; "STALE" means it names an older one.

`claude-garage tasks` prints the same rows as plain text (one block per workspace, `--json` for scripts, `--workspace <name>`, `--local` to skip git fetch and GitHub).

## Decisions

**1. A plugin, not a core feature.** The owner's decision. The plugin uses only plugin API v1; the checks are in "Core changes" below. It is a Rust program in `plugins/openspec-board/` on `garage-plugin-sdk`, like `usage-bar` and `jump-palette`. Split in a library (parsers, collector, linking, waiting list: pure, unit-tested) and a thin `main` (pane UI, widget push, CLI). The pane, the widget tick and `tasks` all call the same collector. `examples/openspec-progress` stays as the reference; its counting rules are copied into the library.

**2. Manifest.**

```toml
id = "openspec-board"
name = "OpenSpec board"
version = "0.1.0"
api_version = 1
platforms = ["macos", "linux"]
description = "Overlay of OpenSpec changes, PR status and what waits on you, per workspace"
requires = ["slots:strip.right"]
permissions = ["sessions.read", "ui.widgets", "ui.panes", "ui.invoke", "ui.navigate"]

[[panes]]
id = "board"
title = "OpenSpec board"
placement = "overlay"
command = ["{bin}", "ui"]

[[actions]]
id = "board"
title = "OpenSpec board"
contexts = ["global"]
pane = "board"
default_key = "o"

[[widgets]]
id = "waiting"
slot = "strip.right"

[[events]]      # redraw the count at once when a session starts or stops needing input
on = "session.status_changed"
command = ["{bin}", "tick"]
[[events]]
on = "daemon.started"
command = ["{bin}", "tick"]
[[events]]
on = "timer"
every_secs = 60
command = ["{bin}", "tick"]

[[cli]]
name = "tasks"
description = "Print OpenSpec changes, PRs and what waits on you"
command = ["{bin}", "cli"]
needs_daemon = true
```

(`[release]` and `[[build]]` are in decision 9.) No dangerous permission: no `sessions.input`, no `core.actions`. `ui.navigate` plus `ui.invoke` is enough for `session.focus`. The exact permission set is fixed when the manifest is written and shown in the install preview; the list above is the target.

**3. Config location and conventions.** Plugin conventions give each plugin one writable state folder (`GARAGE_PLUGIN_STATE_DIR`, `~/.garage/plugins/data/openspec-board/`), kept across updates, and say to treat what the program writes there as a cache. So the folder holds two kinds of files, kept apart:
- `config.toml`: **written only by the user**, read on every run (no restart), optional. The plugin never creates or rewrites it (`claude-garage tasks --print-config-path` shows where it is, and a commented sample is in the README).
- `cache/`: everything the plugin writes (ETags, bodies, tree-id results, last model for the widget). Disposable: deleting it costs one slow refresh. Writes are atomic (temp file, rename) and a read-change-write is under the SDK's `StateLock`, because event runs overlap.

```toml
[workspace.kowboy]                 # a garage workspace name
github = true                      # opt in to gh api for this workspace
repos  = ["~/code/kowboy/api", "~/code/kowboy/web"]   # default: scan workspace dir + direct children
branches = ["feat/*", "integration/*"]                # remote branches to scan; default: auto (decision 5)
fetch = true                       # default true
background_secs = 0                # 0 = refresh only while the pane is open; >= 300 also refreshes from the timer

[workspace.kowboy.pr]
change_title = '^\[(?P<change>[a-z0-9-]+)(?: (?P<task>[\d.]+))?\]'   # default shown
change_branch = '(?P<change>[a-z0-9-]+)'                              # matched inside the head branch name

[workspace.kowboy.comments]        # patterns over PR issue comments, newest match wins
ci     = 'Claude CI \((?P<mode>\w+)\): (?P<status>PASS|FAIL) (?P<sha>[0-9a-f]{7,40})'
review = 'Claude review: (?P<verdict>[A-Za-z _-]+?) (?P<sha>[0-9a-f]{7,40})'
owner_needed = 'owner approval'    # a review verdict matching this lands in "waiting on you"
```

Rules: `~` expands; a path that is not a git repo is a warning row, not an error; an invalid regex disables that one pattern and shows a warning. With no entry for a workspace, `repos` is the workspace dir (if it is a git repo or has `openspec/`) plus its direct child directories that are git repos with `openspec/`. Comment patterns have no default: with none, PRs show without check or verdict columns. The named groups `change`, `task`, `status`, `sha`, `mode`, `verdict` are the only contract. Workspace names and directories come from `GET /api/workspaces`.

**4. Reading OpenSpec.** Per repo, a change is a directory under `openspec/changes/` other than `archive`, with `proposal.md` or `tasks.md`.
- Tasks: list items (`-`, `*`, `+`) beginning `[ ]` or `[x]`/`[X]`, indented sub-tasks included, code fences ignored (the example plugin's rules). `done/total`; the **next open task** is the first unchecked item with its leading number (`2.1`) and text.
- State: `proposal` = no task ticked (or no `tasks.md`); `building` = some but not all ticked; `complete` = all ticked and not archived.
- A change seen in several places (local plus one or more branches) is one row with a `where` list. Display values come from the most advanced source (highest `done`; ties go to local). The row marks when sources disagree.

**5. Remote branches without touching the tree.**
- Refresh: `git -C <repo> fetch --prune origin` when `fetch` is on (at most every 5 minutes while the pane is open, and on manual refresh). Env `GIT_TERMINAL_PROMPT=0`, a 30 s timeout, output ignored; failure marks "fetch failed 12m ago" and the stale refs are still read. Fetch writes only `refs/remotes`, never the working tree, index or current branch.
- Candidate branches: `git for-each-ref --format=%(refname:short) %(committerdate:unix) refs/remotes/origin`. When `branches` is set, only matching globs. Otherwise, branches with a commit in the last 30 days.
- Cheap skip: `git rev-parse origin/<b>:openspec/changes` (the tree id) is compared with the default branch's. Equal means skip. Results are cached by tree id plus blob ids, so an unchanged branch costs one `rev-parse`.
- Read: `git ls-tree -r --name-only origin/<b> openspec/changes/` (filter `proposal.md`, `tasks.md`), then `git show origin/<b>:openspec/changes/<c>/tasks.md`. No checkout, no worktree, no `git stash`, no writes.
- Default branch: `git symbolic-ref refs/remotes/origin/HEAD`, falling back to `main` then `master`.

**6. GitHub, `gh api` REST only.**
- Repo slug from `git remote get-url origin` (github.com forms only; anything else is skipped with a note).
- Open PRs: `gh api -i "repos/{o}/{r}/pulls?state=open&per_page=100"` with `If-None-Match` from the cache. A `304` costs no rate limit and reuses the cached body. Recently merged PRs (last 14 days) come from `state=closed` the same way, used to clear "awaiting approval" and to show `merged`.
- Comments: for each PR that matches a change, `gh api -i "repos/{o}/{r}/issues/{n}/comments?per_page=100"` (follow `Link: rel=next`; read from the last page backwards, stop at the newest match of each pattern). Re-read only when the PR's `updated_at` moved.
- Native checks (optional, `checks = true`): `repos/{o}/{r}/commits/{head}/check-runs`; shown when no `ci` comment pattern is set.
- Linking: title regex first, then branch regex, then the change name as a whole `-`/`/` separated token of the head branch. A PR with no match is listed only with `--all`.
- Head commit: PR `head.sha`. A comment or verdict whose `sha` is a prefix of (or equal to) head is **current**; otherwise **stale** and never counted as "waiting on you" for that reason.
- Rate limit: every response's `x-ratelimit-remaining` is recorded in the cache. Below 200 the poll interval doubles; at 0 until `x-ratelimit-reset` the plugin serves cache and shows "rate limited until 14:05". 403 with a rate-limit body and 401 behave the same, 401 also shows "gh not logged in".
- Offline-safe: `gh` missing, not logged in, no network, or a timeout (15 s) all degrade to cached data with a single status chip (`gh off`, `gh offline 9m`). The pane stays responsive: the collector runs on a worker thread and the screen draws the last model while it works.
- **The plugin environment is cleared** (`PATH`, `HOME`, `USER`, `LANG`, `TERM`, `TMPDIR`, `SHELL` are kept; nothing else). `gh` therefore works through its stored login (`gh auth login`, in `~/.config/gh` or the keychain) and **not** through a `GH_TOKEN` or `GITHUB_TOKEN` exported in the user's shell. The README says so and `gh not logged in` names `gh auth login`. This is a limitation of the plugin sandbox-free model on purpose (plugins never see the user's API keys), not a core gap.

**7. Waiting on you.** Computed from the model, in this order, each row naming the thing to do:
1. `approve proposal`: change in `proposal` state with no merged PR.
2. `review says owner needed`: current-head review verdict matching `owner_needed`.
3. `failed check`: current-head `ci` status `FAIL` (or a failed native check run).
4. `needs input`: a session of the workspace with `status: needs-input` (its `message` shown), linked to a change or not.

A stale verdict or check is displayed `STALE` and never raises an item by itself; a PR with no verdict for its current head shows `review pending`. The strip widget shows the total across all workspaces, built from the live sessions plus the last cached model (decision 10).

**8. Linking sessions to changes.** `GET /api/sessions` already returns `id`, `workspace`, `label`, `dir`, `status`, `message` and `branch`. A session links to change `C` when, in order: its label equals `C` or starts with `C-`; its branch (a worktree branch `garage/<label>` counts by its label, any other branch by name) equals or contains `C` as a token, or equals a PR's head branch for a PR linked to `C`; its `dir` is inside a repo whose current branch matches. First match wins, one change per session. Sessions that link nothing are counted in the workspace line only. The plugin reads sessions from the daemon API and never reads tmux.

**9. Distribution: shipped and updated on its own.**
- Source lives in `plugins/openspec-board/` (same repo, same CI, workspace member through the `plugins/*` glob; `[package.metadata.dist] dist = false` like the other plugin crates).
- Install: `claude-garage plugin install adar2378/claude-garage/plugins/openspec-board` (the installer already takes `owner/repo/subdir`). The user sees the preview (permissions, commands) and grants it; nothing is granted silently.
- The manifest carries `[release]` (`tag = "openspec-board-v{version}"`, a `checksums` asset, assets for `macos-aarch64`, `macos-x86_64`, `linux-x86_64`, `linux-aarch64`) and a `[[build]]` fallback (`cargo build --release`) for people without a matching asset. A CI workflow builds the four assets on a tag `openspec-board-v*` and attaches them with the checksums file (group 10).
- Update: `claude-garage plugin update openspec-board`, independent of the garage version. The plugin declares `requires` for any capability it relies on, so an older garage refuses it with a clear message.
- **Not bundled.** Today `crates/daemon/build.rs` allowlists and embeds **every** `plugins/<id>/` folder except the `hello` fixture, so a folder placed there would silently become part of the `claude-garage` binary. That contradicts "opt-in, own releases". Hence the one core change (below).

**10. Refresh model and idle cost.** A plugin cannot keep a daemon-side loop, and does not need one:
- **The pane process is the consumer.** While the overlay is open its own thread runs: local files every 5 s (stat by mtime; re-read only changed files), sessions every 5 s from `/api/sessions` (and sooner on the pane's own timer), git fetch every 5 min, GitHub every 60 s (backing off per decision 6), `r` forces all three. Closing the pane stops everything: no fetch, no `gh`.
- **The widget tick** (`tick`: on `session.status_changed`, `daemon.started` and a 60 s timer) never runs `git` or `gh` unless `background_secs >= 300` is configured and due. It reads `/api/sessions` (one local request), combines the needs-input count with the waiting items of the last cached model, and `PUT`s the `waiting` widget with `ttl_secs = 300`. The daemon forgets widgets when it restarts, so `daemon.started` redraws from the cache.
- A tick always exits 0 (five failed event runs in a row switch the hooks off); errors go to the log. The pane's cache and the tick's cache are the same files, under `StateLock`.
- Idle cost: one short process a minute plus one on each status change.

**11. The pane UI.** An overlay pane (same terminal kind as an agent tile). Drawn with plain terminal codes and the SDK `tui` kit where it fits; theme-neutral colours (amber only for items that need the human). Keys: `j`/`k` move with wrap; `Enter` focuses the row's session (`POST /api/plugins/ui/action` with `{"action":"session.focus","target":{"session":"<id>"}}`, needs `ui.invoke` and `ui.navigate`) and exits, or with no session opens the PR; `p` opens the PR URL by spawning `open` (macOS) or `xdg-open` (Linux) from the plugin process, and the pane stays open; `c` toggles per-task detail; `r` refresh; `Tab` cycles workspaces (starting at the run context's `focus.workspace`, else the first); `Esc`/`q` closes. Read-only: no key mutates any repo, PR or session. A `session.focus` call is rate limited by the daemon (5 a second) and answers `202`; the wall decides.

**12. The `tasks` command.** `[[cli]] name = "tasks"` (not a reserved core word; plugin CLI names only need to be unique). `needs_daemon = true` so sessions are available; with `--local` or when the daemon call fails it still prints local data and says so. Flags: `--workspace`, `--json`, `--local`, `--all`, `--refresh`, `--print-config-path`.

**13. Privacy.** Everything is computed on the machine. The only network traffic is `git fetch` to the repo's own `origin` (already how the repo works) and `gh api` calls to github.com made by the user's own `gh` login, only for workspaces with `github = true`. No telemetry. The plugin is not sandboxed (plugin API rule); its README names every command it runs: `git` (fetch, rev-parse, ls-tree, show, for-each-ref, symbolic-ref, remote get-url), `gh api` GET, and `open`/`xdg-open`. Tokens are never read or stored by the plugin; `gh` holds them.

## Core changes

The plugin was checked against the plugin docs (`docs/plugins/`), `usage-bar`, `jump-palette` and `examples/openspec-progress`. What it needs and where it comes from:

| Need | Plugin API v1 route | Core change? |
|---|---|---|
| Workspace names and dirs | `GET /api/workspaces` | no |
| Sessions with status, `message`, `branch`, `label` | `GET /api/sessions` | no |
| React to status changes, redraw after restart, slow schedule | events `session.status_changed`, `daemon.started`, `timer` | no |
| Overlay pane, opened by a key or a widget click | `[[panes]]`, `[[actions]]` with `pane`, widget `click` | no |
| "Waiting on you" count in the strip | `[[widgets]]` `strip.right`, `PUT` with `ttl_secs` | no |
| Jump to a session | `POST /api/plugins/ui/action` `session.focus` (`ui.invoke`, `ui.navigate`) | no |
| Open a PR URL | the plugin spawns `open` / `xdg-open` (no sandbox; named in the README) | no |
| Config file and cache | plugin state folder | no |
| `gh` and `git` | child processes (`PATH` and `HOME` are kept) | no |
| **First-party plugin that is not embedded in the garage binary** | none: `build.rs` embeds and allowlists every folder in `plugins/` except `hello` | **yes, C1** |
| Marketing site reading the overview | none: plugins cannot add routes | only if question 8 is yes, **C2** (optional) |

**C1: ship a first-party plugin separately (group 1).** In `crates/daemon/build.rs` the `FIXTURE` check (`id == FIXTURE`) skips the `hello` dev fixture in both the embed step and the allowlist step. Replace the single constant by a short list, `NOT_EMBEDDED = ["hello", "openspec-board"]`, used in both places, with a unit test and an `embed-plugins` check that the payload and `ALLOWLIST` contain no entry for a listed id, and that every other first-party folder is still present. `plugins/` still holds the source (same repo, same CI); the plugin is simply not part of the archive, not seeded by `plugins::bundled`, and not auto-granted. No change to the plugin API, the wall or any route. Without C1 the plugin folder would be compiled into the release binary and be on the allowlist, so C1 must land first.

**C2: optional, off by default (group 12).** If the owner wants the site to read the overview, the smallest change is a generic, plugin-owned read route (for example `PUT /api/plugins/{id}/data` with `ui.widgets`-sized limits and `GET /api/plugins/{id}/data`, both under the existing origin guard) that stores and serves one JSON document per plugin. The overview model then stays plugin code. This is not built unless question 8 is answered yes.

Anything found missing during implementation is added here as a new item with its own task and its own review, never folded into the plugin tasks.

## Risks

- **Comment patterns are the repo's convention, not an API.** Hence configurable, with named groups, absent by default.
- **Large repos / many branches.** Bounded by the tree-id skip, the 30-day filter, caching by tree id, and fetch only while the pane is open.
- **GitHub rate limit.** ETag `304`s are free; polling is lazy and backs off.
- **Linking guesses wrong.** Rules are ordered and documented; the `where`/`sessions` columns make a wrong link visible, and a label that starts with the change name always wins.
- **No `GH_TOKEN` through the cleared environment.** Documented; `gh auth login` is the supported path.
- **A plugin is not sandboxed.** It can read any file the user can. Mitigation: no dangerous permissions, read-only code paths, every command named in the README, GET-only `gh` asserted in a test.
- **Plugin API drift.** API v1 is frozen and additive; the plugin pins `api_version = 1` and uses `requires` for features.
- **Windows.** Not in `platforms` at first (`open`/`xdg-open` paths and `gh` quoting are untested); add later.

## Testing approach

Pure functions (checkbox parser, state, linking, waiting list, stale-vs-head, rate-limit accounting) get unit tests in the plugin library. Git behaviour is tested against temp repos created in the test (a bare "origin", a clone, a branch holding a change the clone lacks; assert the clone's working tree and `git status` are unchanged after a refresh). GitHub is tested with a fake `gh` executable on `PATH` that serves canned responses, ETags, `304`s, rate-limit headers and failures. The pane renders to a plain string buffer for snapshot tests (the SDK `testing` kit and the fake API); widget and action calls are checked against the SDK's fake daemon. C1 is tested in the daemon crate with the existing `plugins::bundled` tests plus `cargo test -p garage-daemon --features embed-plugins --lib -- plugins::bundled`.
