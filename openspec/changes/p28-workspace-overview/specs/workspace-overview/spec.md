## ADDED Requirements

### Requirement: Declaring a workspace's repos (plugin config)
The `openspec-board` plugin SHALL read an optional file `config.toml` in its own state folder (`GARAGE_PLUGIN_STATE_DIR`, `~/.garage/plugins/data/openspec-board/`) on every run, and SHALL NEVER create or write it; because it lives in the state folder, `plugin remove --purge` deletes it. Everything the plugin writes SHALL go to `cache/` in the same folder, created with mode `0700` (files `0600`), because it holds private data: PR titles, branch names and PR comment bodies. The README SHALL say so. A `[workspace.<name>]` table MAY set `repos` (a list of paths, `~` expanded), `github` (boolean, default false), `fetch` (boolean, default true) and `branches` (a list of globs). There SHALL be no setting that makes `git fetch` or `gh` run while the pane is closed. With no table, or no `repos`, the workspace's repos SHALL be the workspace directory (when it is a git repo or contains `openspec/`) plus its direct child directories that are git repos containing `openspec/`. A listed path that is missing or not a git repo SHALL produce a warning on the workspace and SHALL NOT fail the overview. An invalid regex SHALL disable only that pattern and add a warning. A missing or unparsable file SHALL behave as an empty file, with a warning for the latter.

#### Scenario: No config scans children
- **WHEN** workspace `kowboy` has dir `~/code/kowboy`, no config entry, and `~/code/kowboy/api` and `~/code/kowboy/web` are git repos with `openspec/` while `~/code/kowboy/notes` is a plain folder
- **THEN** the overview covers `api` and `web` and not `notes`

#### Scenario: Bad path is a warning
- **WHEN** `repos` lists `~/code/kowboy/gone`, which does not exist
- **THEN** the other repos are still shown and the workspace carries the warning `~/code/kowboy/gone is not a git repo`

#### Scenario: Bad regex disables one pattern
- **WHEN** `comments.review` is not a valid regex
- **THEN** review verdicts are omitted, check results still work, and a warning names `comments.review`

### Requirement: Reading OpenSpec changes
For each repo the plugin SHALL list every directory under `openspec/changes/` other than `archive` that holds `proposal.md` or `tasks.md`. Tasks SHALL be counted as list items (`-`, `*`, `+`, indented sub-tasks included) beginning `[ ]` or `[x]`/`[X]`, ignoring code fences; the result is `done` and `total`, and the next open task is the first unchecked item with its leading number and text. State SHALL be `proposal` when no task is ticked or `tasks.md` is missing, `building` when some but not all are ticked, and `complete` when all are ticked. The plugin SHALL NOT write to any file in the repo.

#### Scenario: Progress and next task
- **WHEN** a change's `tasks.md` has 11 items of which the first 6 are ticked and the seventh is `- [ ] 2.1 debounce input`
- **THEN** the change reads `building`, `6/11`, next open task `2.1 debounce input`

#### Scenario: Checkbox in a code fence ignored
- **WHEN** `tasks.md` shows `- [ ] example` inside a fenced code block
- **THEN** it is not counted

#### Scenario: Archive skipped
- **WHEN** `openspec/changes/archive/2026-11-02-old` exists
- **THEN** it is not listed

#### Scenario: No tasks file is a proposal
- **WHEN** a change has `proposal.md` and no `tasks.md`
- **THEN** it reads `proposal` with `0/0` tasks and no next task

### Requirement: Changes on remote branches
When `fetch` is on, the plugin SHALL run `git fetch --prune --no-auto-gc --no-auto-maintenance --no-write-fetch-head origin` per repo, **only while the overlay pane is open (at most every 5 minutes) and when the user presses `r`** (or runs `tasks --refresh`), with stdin set to null, `GIT_TERMINAL_PROMPT=0`, `GIT_SSH_COMMAND='ssh -o BatchMode=yes'` and a 30 second timeout. The fetch SHALL talk only to the repo's own `origin`, and SHALL write nothing outside `refs/remotes` apart from the objects it downloads: no working tree, index, `HEAD`, local branch, stash, `FETCH_HEAD`, gc or maintenance. A failed fetch SHALL mark the workspace `fetch failed` and SHALL still read the existing remote-tracking refs. Changes SHALL be read from `refs/remotes/origin/<branch>` using `git ls-tree` and `git show` only. A branch whose `openspec/changes` tree id equals the default branch's SHALL be skipped. A change whose name is in the default branch's `openspec/changes/archive/` SHALL be skipped everywhere. Without a `branches` list, only branches with a commit in the last 30 days SHALL be scanned. A change found in several places SHALL be one row listing each place, with values from the most advanced source.

#### Scenario: Change only on an integration branch
- **WHEN** `origin/feat/billing-v2` holds `openspec/changes/billing-v2/` and the local checkout does not
- **THEN** the overview lists `billing-v2` with where `origin/feat/billing-v2`, and the local checkout's working tree and `git status` are identical before and after

#### Scenario: Unchanged branch skipped cheaply
- **WHEN** a branch's `openspec/changes` tree id equals the default branch's
- **THEN** no `git show` is run for it

#### Scenario: Fetch is confined
- **WHEN** a refresh runs the fetch
- **THEN** the command line has `--no-auto-gc --no-auto-maintenance --no-write-fetch-head`, stdin is null, ssh runs with `BatchMode=yes`, and afterwards no ref outside `refs/remotes` and no `FETCH_HEAD` has changed or appeared

#### Scenario: No fetch while closed
- **WHEN** the pane is closed and the widget ticks
- **THEN** no `git fetch` runs

#### Scenario: Archived change skipped
- **WHEN** the default branch has `openspec/changes/archive/2026-11-02-old` and a stale branch still holds `openspec/changes/old/`
- **THEN** `old` is not listed

#### Scenario: Offline fetch
- **WHEN** `git fetch` fails
- **THEN** changes from already-fetched refs are still listed and the workspace shows `fetch failed`

#### Scenario: Sources disagree
- **WHEN** the local `tasks.md` shows 3/10 and `origin/feat/x` shows 6/10 for the same change
- **THEN** one row shows `6/10` and marks that the sources differ

### Requirement: Pull requests tied to changes and tasks
For a workspace with `github = true` (default false) and a github.com `origin`, the plugin SHALL read open pull requests, those merged in the last 14 days, and the changed files of each open PR, using `gh api` REST calls with ETag revalidation. A PR SHALL link to a change when the change name appears, case-insensitively, anywhere in the PR's head branch, its title, or a changed file path under `openspec/changes/<name>/`. When several names match, a diff path wins, then the longest name. There SHALL be no first-token branch regex. The optional `pr.task` pattern SHALL only extract a task number from the title. PRs matching no change SHALL be omitted from the default view. Each linked PR SHALL show number, URL, title, draft flag, head sha and task when the title names one. A workspace without `github = true` SHALL make no GitHub call.

#### Scenario: Linked by title
- **WHEN** an open PR is titled `docs(openspec): propose search-ui` and change `search-ui` exists
- **THEN** the PR appears under `search-ui`

#### Scenario: Linked by a claude branch
- **WHEN** a PR titled `fix typo` has head branch `claude/search-ui-x7q`
- **THEN** it appears under `search-ui` with no task

#### Scenario: Linked by diff path
- **WHEN** a PR titled `wip` on branch `claude/abc123` adds `openspec/changes/search-ui/proposal.md`
- **THEN** it appears under `search-ui`

#### Scenario: Longest name wins
- **WHEN** changes `search` and `search-ui` both exist and a branch is `feat/search-ui`
- **THEN** the PR links to `search-ui`

#### Scenario: Task from the title
- **WHEN** an open PR is titled `[search-ui 1.3] debounce` and `pr.task` is the default
- **THEN** the PR appears under `search-ui` with task `1.3`

#### Scenario: GitHub not enabled
- **WHEN** a workspace has no `github = true`
- **THEN** the overview shows changes with no PR column data, `gh` is never run, and the status reads `gh off`

### Requirement: Check and review results from comments
When `comments.ci` or `comments.review` patterns are configured, the plugin SHALL scan each linked PR's issue comments and take the newest match of each pattern. `ci` yields `status`, `sha` and optional `mode`; `review` yields `verdict` and `sha`. The `comments.owner_needed` pattern SHALL be matched against the **whole body** of the newest comment matching `review`, not against the captured verdict. A result SHALL be **current** when its `sha` equals or is a prefix of the PR's head sha, else **stale**. A stale result SHALL be displayed as stale and SHALL NOT by itself raise a waiting item. With no pattern configured, no comment result is shown. When `checks = true` and no `ci` pattern is set, the conclusion of the head commit's check runs SHALL be used instead.

#### Scenario: Verdict for the current head
- **WHEN** a comment reads `Claude review: APPROVED 9c8d7e6` and the PR head is `9c8d7e6a1b…`
- **THEN** the review shows `APPROVED` marked current

#### Scenario: Verdict for an older commit
- **WHEN** the newest review comment names `5b6c7d8` and the PR head is `77aa11c…`
- **THEN** the review shows its verdict marked STALE, and the PR shows `review pending` for the head

#### Scenario: Newest match wins
- **WHEN** two CI comments match, `FAIL aaa1111` then `PASS bbb2222`
- **THEN** the CI result is `PASS bbb2222`

### Requirement: Waiting on the human
The plugin SHALL list, per workspace, items needing the human: a proposal awaiting approval, a PR whose newest current review comment's whole body matches `comments.owner_needed`, a PR whose current CI status is `FAIL`, and every session of the workspace in `needs-input` (with its message). A proposal is **awaiting approval** when an open PR's diff adds that change's folder; a merged PR, or the change name being in the default branch's `archive/`, clears it. With GitHub off for the workspace there SHALL be no approve-proposal items at all. Each item SHALL name its change and PR or session. Sessions in `needs-input` SHALL appear even when linked to no change.

#### Scenario: Owner approval needed
- **WHEN** the newest review comment is for the PR's head and its body has a line `Owner approval: yes` matching `owner_needed = '(?m)^Owner approval: yes\s*$'`
- **THEN** a waiting item of kind `owner-review` names the PR and its change

#### Scenario: Failed check on head
- **WHEN** a PR's current CI result is `FAIL`
- **THEN** a waiting item of kind `failed-check` names the PR; when the FAIL is stale it does not

#### Scenario: Unlinked session needs input
- **WHEN** a workspace session with no matching change is `needs-input`
- **THEN** it appears in the waiting list with its message

#### Scenario: Approved proposal clears
- **WHEN** a proposal's PR is merged
- **THEN** no `approve-proposal` item remains for it

#### Scenario: Proposal awaiting approval
- **WHEN** GitHub is on and an open PR's diff adds `openspec/changes/billing-v2/proposal.md`
- **THEN** an `approve-proposal` item names `billing-v2` and the PR

#### Scenario: GitHub off
- **WHEN** a workspace has `github` off and a change in state `proposal`
- **THEN** there is no `approve-proposal` item for it

#### Scenario: Archived name
- **WHEN** a change name is in the default branch's `archive/`
- **THEN** no waiting item is raised for it

### Requirement: Sessions linked to changes
A session SHALL link to at most one change of its workspace, using the first rule that matches: its label equals the change name or starts with `<change>-`; its branch (a `garage/<label>` worktree branch counts as its label) equals the change name or contains it, or equals the head branch of a PR linked to the change. Linked sessions SHALL show on the change's row with their status. The plugin SHALL take sessions from the daemon's session list and SHALL NOT read tmux. Only garage sessions are shown; cloud sessions are not garage sessions and are out of scope.

#### Scenario: Linked by label
- **WHEN** session `garage/kowboy/search-ui-ui` exists and change `search-ui` exists
- **THEN** the session shows on the `search-ui` row

#### Scenario: Linked by worktree branch
- **WHEN** a session's branch is `feat/billing-v2` and change `billing-v2` exists
- **THEN** the session shows on the `billing-v2` row

### Requirement: GitHub access is cached, rate-limit aware and offline-safe
GitHub data SHALL be read only through `gh api` REST. Responses SHALL be cached with their ETag under the plugin's `cache/` folder and revalidated with `If-None-Match`. The runner SHALL decide the outcome from the HTTP **status line** in the `gh api -i` output, never from the exit code, because `gh api` exits with status 1 on HTTP 304; the exit code SHALL be consulted only when there is no status line. It SHALL record `X-Ratelimit-Remaining` and `X-Ratelimit-Reset` from every response that has headers, including a 304; below 200 the poll interval SHALL double, and at 0 no call SHALL be made until the reset. A missing `gh`, a login failure, a network failure or a 15 second timeout SHALL serve the cached data and show one status (`gh off`, `gh not logged in`, `gh offline`, `rate limited until <time>`). None of these SHALL block the pane's drawing or make the widget tick fail. A `GH_TOKEN` exported in the user's shell is not visible to the plugin (cleared environment); only the `gh` stored login is used, and `gh not logged in` SHALL say `gh auth login`. Deleting the cache directory SHALL only cause a slower next refresh.

#### Scenario: Unchanged data costs nothing
- **WHEN** GitHub answers `304 Not Modified` to a revalidation and `gh` exits with status 1
- **THEN** the cached body is used, the data is unchanged, no error is shown, and the rate-limit headers of that response are recorded

#### Scenario: Rate limit exhausted
- **WHEN** a response reports `x-ratelimit-remaining: 0`
- **THEN** no further `gh` call is made before the reset time and the cached PRs are shown with `rate limited until <time>`

#### Scenario: Offline
- **WHEN** `gh` fails to reach github.com
- **THEN** cached PRs are shown with `gh offline` and local OpenSpec data is unaffected

### Requirement: Refresh runs only while the pane is open
The plugin's pane process SHALL be the only consumer of `git fetch` and `gh`, apart from an explicit `tasks --refresh`. While the pane is open: local files SHALL be checked every 5 seconds, sessions every 5 seconds, GitHub every 60 seconds and `git fetch` every 5 minutes; a manual refresh does all at once, and the pane SHALL write the resulting model to `cache/`. With the pane closed, no `git fetch` and no `gh` call SHALL run. The strip widget tick SHALL read only the daemon's session list and the last cached model (no repo files, no `git`, no `gh`), SHALL always exit 0, and SHALL set a `ttl_secs` on the widget.

#### Scenario: Idle costs nothing
- **WHEN** the pane has been closed for an hour
- **THEN** the plugin has run no `git fetch` and no `gh` call, only widget ticks

#### Scenario: Edit shows within seconds
- **WHEN** the pane is open and a `tasks.md` checkbox is ticked in an editor
- **THEN** the row's `done/total` updates within about 5 seconds

### Requirement: Overlay pane and status-strip widget
The plugin SHALL declare an overlay pane `board` and an action `openspec-board:board` with the suggested key `o` (a suggestion never overrides a bound key), showing the focused workspace's waiting list and changes as in the design mockup. It SHALL read only the daemon's plugin API and its own cache. `j`/`k` SHALL move the selection with wrap, `Enter` SHALL ask the wall to focus the selected row's session through `POST /api/plugins/ui/action` (`session.focus`) and exit, or with no session open its PR, `p` SHALL open the PR URL from the plugin's own process and keep the pane open, `c` toggles per-task detail, `r` refreshes, `Tab` cycles workspaces, `Esc` closes. No key SHALL change any repo, PR or session. The plugin SHALL also push a `strip.right` widget showing the number of waiting items across workspaces, built from the live session list plus the last cached model. The count SHALL use the `needs_input` colour token only while at least one session is waiting on input, the `warn` token when the count is above zero only because of proposals or failed CI, and dim text at zero (plugins cannot paint amber otherwise). When the cached model is older than 15 minutes or absent, the cached part of the count SHALL be shown as stale (`~`). Clicking the widget opens the pane. The manifest SHALL declare each permission, hook, widget and CLI command only in the task group that implements it, and SHALL request no dangerous permission.

#### Scenario: Jump to the session
- **WHEN** the pane is open, a change row with a linked session is selected and the user presses `Enter`
- **THEN** the plugin posts `session.focus` for that session, the pane closes, and the wall focuses the session, switching workspace and view if needed

#### Scenario: Open the PR
- **WHEN** a PR row is selected and the user presses `p`
- **THEN** the PR URL opens in the browser and the pane stays open

#### Scenario: Empty workspace
- **WHEN** the workspace has no OpenSpec changes
- **THEN** the pane says `no OpenSpec changes in kowboy` and lists any waiting sessions

#### Scenario: Strip count
- **WHEN** two sessions are in `needs-input` and one proposal awaits approval
- **THEN** the strip shows `board 3` in the `needs_input` colour, and with nothing waiting it shows `board 0` dim

#### Scenario: Only proposals and failed CI wait
- **WHEN** no session needs input, one proposal awaits approval and one PR has a failed check
- **THEN** the strip shows `board 2` in the `warn` colour, not `needs_input`

#### Scenario: Old cached model
- **WHEN** the cached model was written 40 minutes ago
- **THEN** the strip shows the cached part of the count marked stale, `board ~3`, and live needs-input sessions are still counted exactly

#### Scenario: Daemon restart
- **WHEN** the daemon restarts and forgets the widget
- **THEN** the `daemon.started` hook redraws it from the cache within seconds

### Requirement: Tasks command
The plugin SHALL declare a CLI command `tasks` (`claude-garage tasks`, `needs_daemon = false`) that prints the overview as plain text, one block per workspace, with `--workspace <name>` to select one, `--json` for the plugin's JSON model, `--local` to skip fetch and GitHub, and `--all` to include unlinked PRs. It SHALL call the daemon API with a short timeout and use it only if it answers, and SHALL otherwise fall back to local-only data, saying so. Without `--refresh` it SHALL run no `git fetch` and no `gh`. Exit code SHALL be 0 whenever the overview was produced, including with warnings.

#### Scenario: Daemon down
- **WHEN** `claude-garage tasks` runs while the daemon is not running
- **THEN** local changes are printed with the note `daemon not running: local data only` and the exit code is 0

#### Scenario: JSON for scripts
- **WHEN** `claude-garage tasks --json --workspace kowboy` runs
- **THEN** stdout is the JSON model for that workspace and nothing else

### Requirement: Read-only and local
The plugin SHALL NOT approve, merge, comment on, label or close a PR, edit any repo file, start, stop or message a session, or open a network connection other than `git fetch` to the repo's own `origin` and `gh api` to github.com for workspaces with `github = true`. It SHALL send no telemetry, and SHALL request no `sessions.input` or `core.actions` permission. It runs only `git`, `gh api` (GET) and the system URL opener. `git fetch` SHALL talk only to the repo's own `origin` and write nothing outside `refs/remotes` apart from downloaded objects.

#### Scenario: Working tree untouched
- **WHEN** an overview refresh runs against a repo with uncommitted edits
- **THEN** the files, index and branch are byte-identical afterwards

#### Scenario: No write calls to GitHub
- **WHEN** any overview refresh runs
- **THEN** every `gh api` call is a `GET`
