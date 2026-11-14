# Design: p28-workspace-overview

## Mockup (the wall overlay, `o`, workspace `kowboy` focused)

```
┌ overview · kowboy ───────────────────────────── 3 waiting on you · fetched 2m ago · gh ok ┐
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
│ j/k move  Enter jump to session  p open PR  r refresh  c change detail  Esc close         │
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

Legend: `●` needs the human now, `◆` PR waiting, `✗` failed check, `◐` working session, `✓` all checks passed. "head" means the verdict or check names the PR's current head commit; "STALE" means it names an older one.

`claude-garage tasks` prints the same rows as plain text (one block per workspace, `--json` for scripts, `--workspace <name>`, `--local` to skip git fetch and GitHub).

## Decisions

**1. A core crate, not a plugin.** The site needs a daemon endpoint, and plugins cannot add routes. The wall already reads everything from the daemon API (spec `tui-wall`), so the overlay does too. The collector is a pure library (`crates/overview`): it takes a snapshot of inputs (directory listings, `git` output, `gh` output, sessions) and returns a model. The daemon owns the I/O loop and the cache; the CLI can run the collector in process when the daemon is down, for local data only. `examples/openspec-progress` stays; its counting rules move into the crate and the example keeps its own copy.

**2. Declaring a project: `~/.garage/overview.toml`.** A project is several repos, so config cannot live in one of them. The file is user-owned, disposable, optional, and read on every refresh (no restart).

```toml
[workspace.kowboy]                 # a garage workspace name
github = true                      # opt in to gh api for this workspace
repos  = ["~/code/kowboy/api", "~/code/kowboy/web"]   # default: scan workspace dir + direct children
branches = ["feat/*", "integration/*"]                # remote branches to scan; default: auto (decision 4)
fetch = true                       # default true

[workspace.kowboy.pr]
change_title = '^\[(?P<change>[a-z0-9-]+)(?: (?P<task>[\d.]+))?\]'   # default shown
change_branch = '(?P<change>[a-z0-9-]+)'                              # matched inside the head branch name

[workspace.kowboy.comments]        # patterns over PR issue comments, newest match wins
ci     = 'Claude CI \((?P<mode>\w+)\): (?P<status>PASS|FAIL) (?P<sha>[0-9a-f]{7,40})'
review = 'Claude review: (?P<verdict>[A-Za-z _-]+?) (?P<sha>[0-9a-f]{7,40})'
owner_needed = 'owner approval'    # a review verdict matching this lands in "waiting on you"
```

Rules: `~` expands; a path that is not a git repo is reported as a warning row, not an error; an invalid regex disables that one pattern and shows a warning. With no entry for a workspace, `repos` is the workspace dir (if it is a git repo or has `openspec/`) plus its direct child directories that are git repos with `openspec/`. Comment patterns have no default: with none, PRs show without check or verdict columns. The named groups `change`, `task`, `status`, `sha`, `mode`, `verdict` are the only contract.

**3. Reading OpenSpec.** Per repo, a change is a directory under `openspec/changes/` other than `archive`, with `proposal.md` or `tasks.md`.
- Tasks: list items (`-`, `*`, `+`) beginning `[ ]` or `[x]`/`[X]`, indented sub-tasks included, code fences ignored (the example plugin's rules). `done/total`; the **next open task** is the first unchecked item, with its leading number (`2.1`) and text.
- State: `proposal` = no task ticked (or no `tasks.md`); `building` = some but not all ticked; `complete` = all ticked and not archived. "Awaiting approval" is a proposal with no merged PR (open question 4).
- A change seen in several places (local plus one or more branches) is one row with a `where` list. Display values come from the most advanced source (highest `done`; ties go to local). The row marks when sources disagree.

**4. Remote branches without touching the tree.**
- Refresh: `git -C <repo> fetch --prune origin` when `fetch` is on (rate: at most every 5 minutes while a consumer looks, and on manual refresh). Env `GIT_TERMINAL_PROMPT=0`, a 30 s timeout, output ignored; failure marks "fetch failed 12m ago" and the stale refs are still read. Fetch writes only `refs/remotes`, never the working tree, index or current branch.
- Candidate branches: `git for-each-ref --format=%(refname:short) %(committerdate:unix) refs/remotes/origin`. When `branches` is set, only matching globs. Otherwise, branches with a commit in the last 30 days.
- Cheap skip: `git rev-parse origin/<b>:openspec/changes` (the tree id) is compared with the default branch's. Equal means no change content, skip. Results are cached by tree id plus blob ids, so an unchanged branch costs one `rev-parse`.
- Read: `git ls-tree -r --name-only origin/<b> openspec/changes/` (filter `proposal.md`, `tasks.md`), then `git show origin/<b>:openspec/changes/<c>/tasks.md`. No checkout, no worktree, no `git stash`, no writes.
- Default branch: `git symbolic-ref refs/remotes/origin/HEAD`, falling back to `main` then `master`.

**5. GitHub, `gh api` REST only.**
- Repo slug from `git remote get-url origin` (github.com forms only; anything else is skipped with a note).
- Open PRs: `gh api -i "repos/{o}/{r}/pulls?state=open&per_page=100"` with `If-None-Match` from the cache. A `304` costs no rate limit and reuses the cached body. Recently merged PRs (last 14 days) come from `state=closed` the same way, used to clear "awaiting approval" and to show `merged`.
- Comments: for each PR that matches a change, `gh api -i "repos/{o}/{r}/issues/{n}/comments?per_page=100"` (follow `Link: rel=next`; read from the last page backwards, stop at the newest match of each pattern). ETag cached; re-read only when the PR's `updated_at` moved.
- Native checks (optional, `checks = true`): `repos/{o}/{r}/commits/{head}/check-runs`; shown when no `ci` comment pattern is set.
- Linking: title regex first, then branch regex, then the change name as a whole `-`/`/` separated token of the head branch. A PR with no match is listed under "other PRs" only with `--all`.
- Head commit: PR `head.sha`. A comment or verdict whose `sha` is a prefix of (or equal to) head is **current**; otherwise **stale** and never counted as "waiting on you" for that reason.
- Rate limit: every response's `x-ratelimit-remaining` is recorded. Below 200 the poll interval doubles; at 0 until `x-ratelimit-reset` the collector serves cache and shows "rate limited until 14:05". 403 with a rate-limit body and 401 behave the same, 401 also shows "gh not logged in".
- Offline-safe: `gh` missing, not logged in, no network, or a timeout (15 s) all degrade to cached data with a single status chip (`gh off`, `gh offline 9m`). The overview never blocks the wall or fails a request because GitHub is unreachable.
- Cache: `~/.garage/cache/overview/` (JSON per repo: ETags, bodies, tree-id results). Disposable; deleting it only costs one slow refresh. Never stored in `state.json`.

**6. Waiting on you.** Computed from the model, in this order, each row naming the thing to do:
1. `approve proposal`: change in `proposal` state with no merged PR.
2. `review says owner needed`: current-head review verdict matching `owner_needed`.
3. `failed check`: current-head `ci` status `FAIL` (or a failed native check run).
4. `needs input`: a linked session with `status: needs-input` (message shown). Sessions not linked to any change still appear here when they need input, under their workspace, so the list is a superset of the triage queue for that workspace.

A stale verdict or check is displayed `STALE` and never raises an item by itself; a PR with no verdict for its current head shows `review pending`.

**7. Linking sessions to changes.** `GET /api/sessions` already returns `branch`, `dir` and `label`. A session links to change `C` when, in order: its label equals `C` or starts with `C-`; its branch (a worktree branch `garage/<label>` counts by its label, any other branch by name) equals or contains `C` as a token, or equals a PR's head branch for a PR linked to `C`; its `dir` is inside a repo whose current branch matches. First match wins, one change per session. Sessions that link nothing are counted in the workspace line only. The collector receives the sessions from the daemon; it never reads tmux.

**8. Refresh cadence.** The collector runs only while a consumer exists: an open overlay, a CLI call, or a client of `/api/overview` seen in the last 60 s (the same "seen" notion as `/api/ui/visibility`). Then: local files every 5 s (stat by mtime; re-read only changed files), sessions on the existing `sessions` event, git fetch every 5 min, GitHub every 60 s (backing off per decision 5), `r` forces everything. With no consumer nothing runs: no fetch, no `gh`, zero idle cost. A completed refresh emits `event: overview` so open clients redraw.

**9. API.** `GET /api/overview[?workspace=<name>]` returns the model (below). `POST /api/overview/refresh {workspace?}` forces a refresh and answers `202`. Both under the daemon's existing loopback binding and origin guard. The model is JSON, versionless for now, added to the daemon-api spec; fields are additive only.

```jsonc
{ "generatedAt": 1763100000000,
  "github": {"state":"ok|off|offline|ratelimited|unauth", "until": null},
  "workspaces": [{
    "name":"kowboy", "warnings":["~/code/kowboy/old is not a git repo"],
    "fetch": {"at":1763099880000,"ok":true},
    "waiting":[{"kind":"approve-proposal|owner-review|failed-check|needs-input","change":"billing-v2","ref":"api#212","text":"…","since":null}],
    "changes":[{
      "name":"search-ui","repo":"web","state":"building",
      "where":[{"kind":"local"},{"kind":"branch","ref":"origin/feat/search-ui"}],
      "tasks":{"done":6,"total":11,"next":{"id":"2.1","text":"debounce input"}},
      "prs":[{"repo":"web","number":88,"url":"…","title":"[search-ui 1.3] …","task":"1.3","draft":false,"head":"e4f5a6b",
              "ci":{"status":"FAIL","mode":"full","sha":"e4f5a6b","current":true},
              "review":{"verdict":"APPROVED","sha":"9c8d7e6","current":false}}],
      "sessions":["garage/kowboy/search-ui-ui"] }]
  }] }
```

**10. The wall overlay.** `wall/src/ui/overview.rs`, a modal like `triage.rs` (same `centered_rect`/`modal_width`, `wrap_selection`, theme colours; amber only for items that need the human). It fetches `/api/overview?workspace=<focused>` when opened and on each `overview` event; it never shells out. Keys inside: `j`/`k` move, `Enter` focuses the row's session (switching workspace and view, like `a`-jump, via the existing jump action) or, with no session, opens the PR; `p` opens the row's PR URL through the existing OSC 8 / opener path; `c` toggles per-task detail for the change; `r` refresh; `Tab` cycles workspaces; `Esc` closes. Open question 5 picks the open key. The overlay is read-only: no key mutates anything.

**11. Privacy.** Everything is computed on the machine. The only network traffic is `git fetch` to the repo's own `origin` (already how the repo works) and `gh api` calls to github.com made by the user's own `gh` login, only for workspaces with `github = true`. No new listener, no telemetry, no garage-run service. The endpoint stays on `127.0.0.1`. The README Privacy section gains a sentence naming `git fetch` and opt-in `gh`. Tokens are never read or stored by garage; `gh` holds them.

## Risks

- **Comment patterns are the repo's convention, not an API.** Hence configurable, with named groups, and absent by default.
- **Large repos / many branches.** Bounded by the tree-id skip, the 30-day filter, caching by tree id, and fetch only on demand.
- **GitHub rate limit.** ETag `304`s are free; the poll is lazy and backs off.
- **Linking guesses wrong.** Rules are ordered and documented; the `where`/`sessions` columns make a wrong link visible, and a label that starts with the change name always wins.
- **Windows.** `gh` and `git` exist there; the collector uses no tmux. Marked preview like the rest.

## Testing approach

Pure functions (checkbox parser, state, linking, waiting list, stale-vs-head, rate-limit accounting) get unit tests. Git behaviour is tested against temp repos created in the test (a bare "origin", a clone, a branch holding a change the clone lacks; assert the clone's working tree and `git status` are unchanged after a refresh). GitHub is tested with a fake `gh` executable on `PATH` that serves canned responses, ETags, `304`s, rate-limit headers and failures. Wall rendering uses the existing insta snapshot pattern; the API goes through the contract suite.
