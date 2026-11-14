# Design: p28-workspace-overview (the `openspec-board` plugin)

## Mockup (the overlay pane, workspace `kowboy` focused)

```
┌ openspec-board · kowboy ─────────────────────── 3 waiting on you · fetched 2m ago · gh ok ┐
│ WAITING ON YOU                                                                            │
│ ▸ ● proposal  billing-v2          approve proposal            api#210       PR open       │
│   ◆ PR api#212  [billing-v2 2.1]  review: owner approval needed   for head a1b2c3d        │
│   ✗ PR web#88   [search-ui 1.3]   CI (full): FAIL e4f5a6b (head)                          │
│   ● session  billing-v2-ui        needs input: "Allow Bash(npm test)?"        4m          │
│                                                                                           │
│ CHANGES                                    tasks             next open       PRs  sessions │
│   billing-v2     proposal    api  origin/feat/billing-v2   0/14  1.1 schema      1   ◐ 1  │
│   search-ui      building    web  local                    6/11  2.1 debounce  2 ● ◆   ◐ 2 │
│   ├ [search-ui 1.3] web#88  CI FAIL e4f5a6b (head) · review n/a                            │
│   └ [search-ui 2.1] web#91  CI PASS 9c8d7e6 (head) · review APPROVED 9c8d7e6 (head)        │
│   audit-log      building    api  local + origin/feat/audit   9/9  –  complete     1 ✓   ○ 0 │
│   └ [audit-log 3.2] api#209 CI PASS 77aa11c · review PASS 5b6c7d8 (STALE: head is 77aa11c) │
│                                                                                           │
│ j/k move  Enter jump to session  p open PR  r refresh  c change detail  Tab workspace  Esc│
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

The status strip (right side) while the pane is closed: `board 3` in the `needs_input` colour while a session waits on input, in `warn` when only proposals or failed CI wait, dim `board 0` otherwise; `board ~3` when the cached model is old. Clicking it opens the pane.

Legend: `●` needs the human now, `◆` PR waiting, `✗` failed check, `◐` working session, `✓` all checks passed. "head" means the verdict or check names the PR's current head commit; "STALE" means it names an older one.

`claude-garage tasks` prints the same rows as plain text (one block per workspace, `--json` for scripts, `--workspace <name>`, `--local` to skip fetch and GitHub, even with `--refresh`).

## Who runs what, and what each part reads

One table so the rest of the document is consistent. Only the **pane process** and the explicit **`tasks --refresh`** run `git fetch` or `gh`. Nothing else ever does.

| Part | Runs `git`? | Runs `gh`? | Reads | Writes |
|---|---|---|---|---|
| Pane (while open; `r` forces) | yes: fetch, rev-parse, ls-tree, show, for-each-ref, symbolic-ref, remote get-url | yes: `gh api` GET | repo files, `GET /api/sessions`, `GET /api/workspaces`, `cache/` | `cache/` only |
| `tasks` | read-only git (no fetch) unless `--refresh` | only with `--refresh` and `github = true` | repo files, `cache/`, the daemon only if it answers | `cache/` only with `--refresh` |
| Widget `tick` | no | no | `GET /api/sessions` and the last cached model in `cache/` (no repo files) | `PUT` of the widget; nothing in `cache/` |
| `ui/action` `session.focus`, `open`/`xdg-open` | no | no | n/a | n/a |

## Decisions

**1. A plugin, not a core feature, and not under `plugins/`.** The owner's decision. It uses only plugin API v1 (checked in "Core changes" below). It is a Rust crate in `extras/openspec-board/` with its own `[workspace]` table, built like `examples/openspec-progress`. It is split into a library (parsers, collector, linking, waiting list: pure, unit-tested) and a thin `main` (pane UI, widget push, CLI). The pane, the widget tick and `tasks` all use the same model types. `examples/openspec-progress` stays as the reference; its counting rules are copied into the library.

**2. Manifest, built up group by group.** Every hook, CLI command, widget, pane and permission is declared in the task group that implements it, never earlier. This is the final shape, with the group in the comments:

```toml
id = "openspec-board"
name = "OpenSpec board"
version = "0.1.0"           # the plugin's own version, also the crate's version
api_version = 1
platforms = ["macos", "linux"]
description = "Overlay of OpenSpec changes, PR status and what waits on you, per workspace"
permissions = ["ui.panes"]                       # group 1
# group 4 adds: "sessions.read", "ui.invoke", "ui.navigate"
# group 5 adds: "ui.widgets"

[[build]]                                         # group 1: build from source
command = ["cargo", "build", "--release"]

[[panes]]                                         # group 1
id = "board"
title = "OpenSpec board"
placement = "overlay"
command = ["{plugin_dir}/target/release/openspec-board", "ui"]

[[actions]]                                       # group 1
id = "board"
title = "OpenSpec board"
contexts = ["global"]
pane = "board"
default_key = "o"

# group 5: requires = ["slots:strip.right"], and
# [[widgets]] id = "waiting", slot = "strip.right"
# [[events]] on = "session.status_changed" / "daemon.started" / "timer" (every_secs = 60)
#            command = ["{plugin_dir}/target/release/openspec-board", "tick"]

# group 6:
# [[cli]] name = "tasks", command = [".../openspec-board", "cli"], needs_daemon = false
```

There is no `{bin}` and no `[release]` table: `{bin}` means the downloaded release binary, which this plugin does not have yet (decision 9). Commands point at the real build path `{plugin_dir}/target/release/openspec-board`, exactly as the example does. The crate has its own `version` (starts at `0.1.0`), its own `Cargo.lock` and no tie to garage's workspace version. No dangerous permission: no `sessions.input`, no `core.actions`. `ui.navigate` plus `ui.invoke` is enough for `session.focus`. The permission set is shown in the install preview.

**3. Config location and conventions.** Plugin conventions give each plugin one writable state folder (`GARAGE_PLUGIN_STATE_DIR`, `~/.garage/plugins/data/openspec-board/`), kept across updates, like `alerts`. **`plugin remove --purge` deletes that folder, including `config.toml`**; the README says so and suggests keeping a copy. The folder holds two kinds of files, kept apart:
- `config.toml`: **written only by the user**, read on every run (no restart), optional. The plugin never creates or rewrites it (`tasks --print-config-path` shows where it is, and a commented sample is in the README).
- `cache/`: everything the plugin writes (ETags, bodies, tree-id results, the last model for the widget). Disposable: deleting it costs one slow refresh. Writes are atomic (temp file, rename) and a read-change-write is under the SDK's `StateLock`, because event runs overlap. **`cache/` holds private data: PR titles, branch names and PR comment bodies (review text).** It is created with mode `0700` (files `0600`), is never sent anywhere, and the README privacy note says to treat it like the repos themselves.

```toml
[workspace.kowboy]                 # a garage workspace name
github = true                      # opt in to gh api for this workspace (default false)
repos  = ["~/code/kowboy/api", "~/code/kowboy/web"]   # default: scan workspace dir + direct children
branches = ["feat/*", "integration/*"]                # remote branches to scan; default: auto (decision 5)
fetch = true                       # default true; runs only while the pane is open or on r

[workspace.kowboy.pr]
task = '\[(?P<change>[a-z0-9-]+)(?: (?P<task>[\d.]+))?\]'   # only extracts the task number from a title; default shown

[workspace.kowboy.comments]        # patterns over PR issue comments, newest match wins
ci     = 'Claude CI \((?P<mode>\w+)\): (?P<status>PASS|FAIL) (?P<sha>[0-9a-f]{7,40})'
review = 'Claude review: (?P<verdict>[A-Za-z _-]+?) (?P<sha>[0-9a-f]{7,40})'
owner_needed = '(?m)^Owner approval: yes\s*$'   # matched against the WHOLE body of the newest review comment
```

Rules: `~` expands; a path that is not a git repo is a warning row, not an error; an invalid regex disables that one pattern and shows a warning. With no entry for a workspace, `repos` is the workspace dir (if it is a git repo or has `openspec/`) plus its direct child directories that are git repos with `openspec/`. Comment patterns have no default: with none, PRs show without check or verdict columns. The named groups `task`, `status`, `sha`, `mode`, `verdict` are the only contract. There is no `change_branch` and no first-token branch regex: linking is decision 6. Workspace names and directories come from `GET /api/workspaces`.

`owner_needed` is matched against the **whole comment body**, not against the captured `verdict`, because the reviewer's format puts `Owner approval: yes` on its own line, apart from the verdict line. The comment examined is the newest comment matching `review`; it counts only when that comment is current (its `sha` is the head).

**4. Reading OpenSpec.** Per repo, a change is a directory under `openspec/changes/` other than `archive`, with `proposal.md` or `tasks.md`.
- Tasks: list items (`-`, `*`, `+`) beginning `[ ]` or `[x]`/`[X]`, indented sub-tasks included, code fences ignored (the example plugin's rules). `done/total`; the **next open task** is the first unchecked item with its leading number (`2.1`) and text.
- State: `proposal` = no task ticked (or no `tasks.md`); `building` = some but not all ticked; `complete` = all ticked and not archived.
- Archived names: any change whose name appears in the default branch's `openspec/changes/archive/` (folders are `<date>-<name>`) is skipped everywhere, so a stale branch copy of an archived change never reappears.
- A change seen in several places (local plus one or more branches) is one row with a `where` list. Display values come from the most advanced source (highest `done`; ties go to local). The row marks when sources disagree.

**5. Remote branches without touching the tree.**
- Refresh: `git -C <repo> fetch --prune --no-auto-gc --no-auto-maintenance --no-write-fetch-head origin` when `fetch` is on, **only while the overlay is open (at most every 5 minutes) and when the user presses `r`** (or runs `tasks --refresh`). Child process: **stdin is null**, `GIT_TERMINAL_PROMPT=0`, `GIT_SSH_COMMAND='ssh -o BatchMode=yes'` (no prompt for a passphrase or host key), a 30 s timeout, output ignored. Failure marks "fetch failed 12m ago" and the stale refs are still read.
- What a fetch writes: remote-tracking refs under `refs/remotes/origin` (new and pruned) and the objects it downloads into the object store, as every fetch does. **No writes outside `refs/remotes`** besides those objects: no working tree, index, `HEAD`, local branch, stash, `FETCH_HEAD`, gc or maintenance run. `--no-write-fetch-head` removes the `FETCH_HEAD` write; the two `--no-auto-*` flags stop a fetch from starting `gc` or background maintenance in the user's repo.
- **Fetch talks only to the user's own remote**: `origin` as configured in that repo, the same host the user already pushes to. The plugin names no other remote and no URL of its own.
- Candidate branches: `git for-each-ref --format=%(refname:short) %(committerdate:unix) refs/remotes/origin`. When `branches` is set, only matching globs. Otherwise, branches with a commit in the last 30 days.
- Cheap skip: `git rev-parse origin/<b>:openspec/changes` (the tree id) is compared with the default branch's. Equal means skip. Results are cached by tree id plus blob ids, so an unchanged branch costs one `rev-parse`.
- Read: `git ls-tree -r --name-only origin/<b> openspec/changes/` (filter `proposal.md`, `tasks.md`), then `git show origin/<b>:openspec/changes/<c>/tasks.md`. No checkout, no worktree, no `git stash`, no writes.
- Default branch: `git symbolic-ref refs/remotes/origin/HEAD`, falling back to `main` then `master`. Its `archive/` listing is read with `git ls-tree` (decision 4).

**6. GitHub, `gh api` REST only, off by default.**
- A workspace makes no GitHub call unless it has `github = true`. First use prints one line saying the plugin will run `gh api` against github.com for that workspace's repos. With GitHub off the pane says `gh off` and shows **no approve-proposal items at all** (decision 7).
- Repo slug from `git remote get-url origin` (github.com forms only; anything else is skipped with a note).
- Open PRs: `gh api -i "repos/{o}/{r}/pulls?state=open&per_page=100"` with `If-None-Match` from the cache. Recently merged PRs (last 14 days) come from `state=closed` the same way, used to show `merged`.
- **`gh api` exits with status 1 on HTTP 304** (and on other 4xx and 5xx), so the plugin never uses the exit code to decide what happened. With `-i` the output starts with the status line (`HTTP/2.0 304 Not Modified`) and the headers; the runner parses the **status line**: `200` use the new body and ETag, `304` reuse the cached body, `401`/`403`/`404`/`5xx` as in the next bullets. The exit code is looked at only when there is no status line at all (the process never reached GitHub: missing `gh`, timeout, no network). The headers are parsed on every response, including the non-zero-exit ones, so **`X-Ratelimit-Remaining`** and **`X-Ratelimit-Reset`** are recorded even from a 304.
- **Changed files of a PR:** `gh api -i "repos/{o}/{r}/pulls/{n}/files?per_page=100"` for each open PR, re-read only when its head sha moved. A PR is **awaiting approval** for change `C` when this list contains a file with status `added` under `openspec/changes/C/` (decision 7). The same paths feed linking.
- Comments: for each PR linked to a change, `gh api -i "repos/{o}/{r}/issues/{n}/comments?per_page=100"` (follow `Link: rel=next`; read from the last page backwards, stop at the newest match of each pattern). Re-read only when the PR's `updated_at` moved.
- Native checks (optional, `checks = true`): `repos/{o}/{r}/commits/{head}/check-runs`; shown when no `ci` comment pattern is set.
- **Linking** (a PR to a change): the change name matched, case-insensitively, as a substring **anywhere** in the PR's head branch, its title, or a changed file path (`openspec/changes/<name>/...`). Examples: branch `claude/openspec-search-ui-x7q` and title `docs(openspec): propose search-ui` both link to `search-ui`. When several change names match, a diff path under `openspec/changes/<name>/` wins, then the longest name wins (so `search-ui` beats `search`). A PR linking nothing is listed only with `--all`. The title task pattern (`pr.task`) only extracts a task number such as `1.3`.
- Head commit: PR `head.sha`. A comment or verdict whose `sha` is a prefix of (or equal to) head is **current**; otherwise **stale** and never counted as "waiting on you" for that reason.
- Rate limit: `X-Ratelimit-Remaining` is recorded in the cache on every response. Below 200 the poll interval doubles; at 0 until `X-Ratelimit-Reset` the plugin serves cache and shows "rate limited until 14:05". 403 with a rate-limit body behaves the same; 401 shows "gh not logged in".
- Offline-safe: `gh` missing, not logged in, no network, or a timeout (15 s) all degrade to cached data with a single status chip (`gh off`, `gh offline 9m`). The pane stays responsive: the collector runs on a worker thread and the screen draws the last model while it works.
- **The plugin environment is cleared** (`PATH`, `HOME`, `USER`, `LANG`, `TERM`, `TMPDIR`, `SHELL` are kept; nothing else). `gh` therefore works through its stored login (`gh auth login`) and **not** through a `GH_TOKEN` or `GITHUB_TOKEN` exported in the user's shell. The README says so and `gh not logged in` names `gh auth login`.

**7. Waiting on you.** Computed from the model, in this order, each row naming the thing to do:
1. `approve proposal`: a proposal is awaiting approval when an **open PR's diff adds that change's folder** (decision 6). Needs GitHub: **with `github` off there are no approve-proposal items at all** (an unmerged proposal cannot be told apart from an idle local draft without a PR). A merged PR, or a change name found in the default branch's `archive/`, clears it.
2. `review says owner needed`: the newest current-head review comment whose whole body matches `owner_needed`.
3. `failed check`: current-head `ci` status `FAIL` (or a failed native check run).
4. `needs input`: a session of the workspace with `status: needs-input` (its `message` shown), linked to a change or not.

A stale verdict or check is displayed `STALE` and never raises an item by itself; a PR with no verdict for its current head shows `review pending`.

**8. Linking sessions to changes.** `GET /api/sessions` already returns `id`, `workspace`, `label`, `dir`, `status`, `message` and `branch`. A session links to change `C` when, in order: its label equals `C` or starts with `C-`; its branch (a worktree branch `garage/<label>` counts by its label, any other branch by name) equals or contains `C`, or equals a PR's head branch for a PR linked to `C`; its `dir` is inside a repo whose current branch matches. First match wins, one change per session. Sessions that link nothing are counted in the workspace line only. The plugin reads sessions from the daemon API and never reads tmux. Sessions are garage sessions only; cloud sessions are not garage sessions and are out of scope.

**9. Distribution: build from source, opt-in.**
- Source lives in `extras/openspec-board/`, outside the garage workspace (the root `Cargo.toml` members are `wall`, `crates/*` and `plugins/*`; `extras/` is none of them, and the crate carries its own `[workspace]`, as `examples/openspec-progress` does). So it is not in `plugins/`, the daemon's `build.rs` neither embeds nor allowlists it, and `plugins::bundled` never seeds it. **No core change.**
- Install: `claude-garage plugin install adar2378/claude-garage/extras/openspec-board`. The installer takes `owner/repo/subdir`; the user sees the preview (permissions, commands) and grants it. `[[build]]` runs `cargo build --release`, so a Rust toolchain is required. Group 7 checks that the SDK path dependency (`../../crates/plugin-sdk`) resolves after the installer's fetch; if not, the crate depends on the SDK by git revision instead.
- Update: re-run `claude-garage plugin update openspec-board`, which rebuilds from the repository.
- **No release pipeline for now.** The manifest has no `[release]` table and the repo gets no plugin release tag: garage's own release is cargo-dist, which triggers on any tag containing `X.Y.Z`, and `scripts/check-release-tag.sh` fails unless the tag names the workspace version. A tag such as `openspec-board-v0.1.0` would start garage's own release or fail its gate. A release path (a tag scheme the garage release ignores, assets and checksums, `{bin}`) is **later work**, recorded in `LATER.md`.

**10. Refresh model and idle cost.** A plugin cannot keep a daemon-side loop, and does not need one:
- **The pane process is the consumer.** While the overlay is open its own thread runs: local files every 5 s (stat by mtime; re-read only changed files), sessions every 5 s from `/api/sessions`, git fetch every 5 min, GitHub every 60 s (backing off per decision 6), `r` forces all three. After each pass it writes the model to `cache/`. Closing the pane stops everything: no fetch, no `gh`. There is no background refresh setting.
- **The widget tick** (`tick`: on `session.status_changed`, `daemon.started` and a 60 s timer) never runs `git` or `gh` and never reads repo files. It reads `/api/sessions` (one local request) and the last cached model, and `PUT`s the `waiting` widget with `ttl_secs = 300`. The daemon forgets widgets when it restarts, so `daemon.started` redraws from the cache.
- **Colour.** Plugins cannot paint amber at will; the `needs_input` token is amber only while a session waits on input. The widget uses `needs_input` for the count **only when at least one live session is in `needs-input`**, `warn` when the count is above zero because of proposals or failed CI only, and plain dim text at zero.
- **Stale count.** The cached model records when the pane last refreshed it. Older than 15 minutes (or absent), the cached part of the count is shown as stale, `board ~3`, and the age is shown only in the pane's title (`updated 40m ago`), because a strip widget has no tooltip; live session counts are still exact.
- A tick always exits 0 (five failed event runs in a row switch the hooks off); errors go to the log. Idle cost: one short process a minute plus one on each status change.

**11. The pane UI.** An overlay pane (same terminal kind as an agent tile). Drawn with plain terminal codes and the SDK `tui` kit where it fits; theme-neutral colours (the `needs_input` look only for items that need the human). Keys: `j`/`k` move with wrap; `Enter` focuses the row's session (`POST /api/plugins/ui/action` with `{"action":"session.focus","target":{"session":"<id>"}}`, needs `ui.invoke` and `ui.navigate`) and exits, or with no session opens the PR; `p` opens the PR URL by spawning `open` (macOS) or `xdg-open` (Linux) from the plugin process, and the pane stays open; `c` toggles per-task detail; `r` refresh; `Tab` cycles workspaces (starting at the run context's `focus.workspace`, else the first); `Esc`/`q` closes. Read-only: no key mutates any repo, PR or session. A `session.focus` call is rate limited by the daemon (5 a second) and answers `202`; the wall decides.

**The suggested key `o`.** The wall's default garage-layer keys are taken: `1`-`9`, `[`, `]`, `a`, `A`, `n`, `N`, `m`, `R`, `x`, `w`, `X`, `K`, `d`, `D`, `p`, `r`, `I`, `t`, `b`, `<`, `>`, `?`, `q`, Tab, Enter and Shift+Up/Down (from `Keymap::defaults` in `wall/src/keymap.rs`). First-party plugins suggest `H` (closed-chats), `/` (jump-palette), `P` (pit-pet), `s` and `S` (quick-send). `o` is free. A manifest suggestion never overrides a bound key; users can rebind with `plugin bind`.

**12. The `tasks` command.** `[[cli]] name = "tasks"` (not a reserved core word; plugin CLI names only need to be unique). **`needs_daemon = false`**: the command must work with the daemon down. It tries `GET /api/sessions` and `GET /api/workspaces` with a short timeout and uses the answer only if there is one; otherwise it falls back to local-only data (workspaces from the config, no sessions) and says so. Flags: `--workspace`, `--json`, `--local` (skip fetch and GitHub, even with `--refresh`), `--all`, `--refresh`, `--print-config-path`. Without `--refresh` it uses the cache for GitHub data and runs no fetch.

**13. Privacy.** Everything is computed on the machine. The only network traffic is `git fetch` to the repo's own `origin` (already how the repo works; it talks only to the user's own remote) and `gh api` calls to github.com made by the user's own `gh` login, only for workspaces with `github = true`. No telemetry. The plugin is not sandboxed (plugin API rule); its README names every command it runs: `git` (fetch, rev-parse, ls-tree, show, for-each-ref, symbolic-ref, remote get-url), `gh api` GET, and `open`/`xdg-open`. Tokens are never read or stored by the plugin; `gh` holds them. **`cache/` stores private PR titles, branch names and comment bodies on disk** (mode `0700`); the README says to delete it before sharing a home directory or a backup.

## Core changes

None. The plugin was checked against the plugin docs (`docs/plugins/`), `usage-bar`, `jump-palette` and `examples/openspec-progress`. What it needs and where it comes from:

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
| Not embedded in the garage binary | a folder outside `plugins/` with its own `[workspace]` | no |

Dropped from the earlier revision: the `NOT_EMBEDDED` list in `crates/daemon/build.rs` (the former C1) is unnecessary because the plugin is not under `plugins/`; the generic plugin data route for the marketing site (the former C2) is dropped because the owner decided against a site route.

Anything found missing during implementation is added here as a new item with its own task and its own review, never folded into the plugin tasks.

## Risks

- **Comment patterns are the repo's convention, not an API.** Hence configurable, with named groups, absent by default.
- **Large repos / many branches.** Bounded by the tree-id skip, the 30-day filter, caching by tree id, and fetch only while the pane is open.
- **GitHub rate limit.** ETag `304`s are free; polling is lazy and backs off. The per-PR files call is re-read only when the head moves.
- **Linking guesses wrong.** Name-anywhere matching can over-link on short names. Mitigation: diff paths win, then the longest name; the `where`/`sessions` columns make a wrong link visible.
- **Stale widget count.** The tick cannot refresh GitHub data; it says so with the `~` mark.
- **No `GH_TOKEN` through the cleared environment.** Documented; `gh auth login` is the supported path.
- **A plugin is not sandboxed.** It can read any file the user can. Mitigation: no dangerous permissions, read-only code paths, every command named in the README, GET-only `gh` asserted in a test.
- **Private data in `cache/`.** PR titles and comment bodies sit on disk. Mitigation: `0700`/`0600`, documented, deletable.
- **Config lost on `--purge`.** Documented in the README.
- **Build from source needs a Rust toolchain**, and the SDK path dependency must resolve after install (checked in group 7).
- **Plugin API drift.** API v1 is frozen and additive; the plugin pins `api_version = 1` and uses `requires` for features.
- **Windows.** Not in `platforms` at first; add later.

## Testing approach

Pure functions (checkbox parser, state, linking, waiting list, stale-vs-head, rate-limit accounting, `gh -i` status parsing) get unit tests in the plugin library. **Every group's tests build their own temp fixture** (temp dirs, temp repos, a bare "origin" and a clone, a fake `gh` on `PATH`, the SDK fake daemon) and never depend on a real workspace, a real network or an earlier group's output. Git behaviour is tested against temp repos created in the test (a branch holding a change the clone lacks; assert the clone's working tree, `git status`, `HEAD` and refs outside `refs/remotes` are unchanged after a refresh, and that no `FETCH_HEAD` is written). GitHub is tested with a fake `gh` executable that serves canned responses, ETags, `304`s **with exit status 1 and a status line**, rate-limit headers and failures. The pane renders to a plain string buffer for snapshot tests; widget and action calls are checked against the SDK's fake daemon.

Test commands: the crate is its own workspace, so every command is `cargo test --manifest-path extras/openspec-board/Cargo.toml <filter>` with **one** filter (a module path such as `config::`), or `-- --exact <full test name>` for a single test. `cargo test` accepts one positional filter, so a group with several areas lists one command per area.
