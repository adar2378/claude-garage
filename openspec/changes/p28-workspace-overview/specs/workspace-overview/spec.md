## ADDED Requirements

### Requirement: Declaring a workspace's repos
garage SHALL read an optional file `~/.garage/overview.toml` (honouring `GARAGE_DIR`) on every refresh. A `[workspace.<name>]` table MAY set `repos` (a list of paths, `~` expanded), `github` (boolean, default false), `fetch` (boolean, default true) and `branches` (a list of globs). With no table, or no `repos`, the workspace's repos SHALL be the workspace directory (when it is a git repo or contains `openspec/`) plus its direct child directories that are git repos containing `openspec/`. A listed path that is missing or not a git repo SHALL produce a warning on the workspace and SHALL NOT fail the overview. An invalid regex SHALL disable only that pattern and add a warning. A missing or unparsable file SHALL behave as an empty file, with a warning for the latter.

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
For each repo the overview SHALL list every directory under `openspec/changes/` other than `archive` that holds `proposal.md` or `tasks.md`. Tasks SHALL be counted as list items (`-`, `*`, `+`, indented sub-tasks included) beginning `[ ]` or `[x]`/`[X]`, ignoring code fences; the result is `done` and `total`, and the next open task is the first unchecked item with its leading number and text. State SHALL be `proposal` when no task is ticked or `tasks.md` is missing, `building` when some but not all are ticked, and `complete` when all are ticked. The overview SHALL NOT write to any file in the repo.

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
When `fetch` is on, the overview SHALL run `git fetch --prune origin` per repo at most every 5 minutes while a consumer exists, and on manual refresh, with prompts disabled and a 30 second timeout. A failed fetch SHALL mark the workspace `fetch failed` and SHALL still read the existing remote-tracking refs. Changes SHALL be read from `refs/remotes/origin/<branch>` using `git ls-tree` and `git show` only; the working tree, index, current branch, stash and worktrees SHALL NOT be modified. A branch whose `openspec/changes` tree id equals the default branch's SHALL be skipped. Without a `branches` list, only branches with a commit in the last 30 days SHALL be scanned. A change found in several places SHALL be one row listing each place, with values from the most advanced source.

#### Scenario: Change only on an integration branch
- **WHEN** `origin/feat/billing-v2` holds `openspec/changes/billing-v2/` and the local checkout does not
- **THEN** the overview lists `billing-v2` with where `origin/feat/billing-v2`, and the local checkout's working tree and `git status` are identical before and after

#### Scenario: Unchanged branch skipped cheaply
- **WHEN** a branch's `openspec/changes` tree id equals the default branch's
- **THEN** no `git show` is run for it

#### Scenario: Offline fetch
- **WHEN** `git fetch` fails
- **THEN** changes from already-fetched refs are still listed and the workspace shows `fetch failed`

#### Scenario: Sources disagree
- **WHEN** the local `tasks.md` shows 3/10 and `origin/feat/x` shows 6/10 for the same change
- **THEN** one row shows `6/10` and marks that the sources differ

### Requirement: Pull requests tied to changes and tasks
For a workspace with `github = true` and a github.com `origin`, the overview SHALL read open pull requests and those merged in the last 14 days using `gh api` REST calls with ETag revalidation. A PR SHALL link to a change by title match of `pr.change_title` (default `^\[(?P<change>[a-z0-9-]+)(?: (?P<task>[\d.]+))?\]`), else by `pr.change_branch`, else by the change name as a whole token of the head branch. PRs matching no change SHALL be omitted from the default view. Each linked PR SHALL show number, URL, title, draft flag, head sha and task when the title names one. A workspace without `github = true` SHALL make no GitHub call.

#### Scenario: Linked by title prefix
- **WHEN** an open PR is titled `[search-ui 1.3] debounce` and change `search-ui` exists
- **THEN** the PR appears under `search-ui` with task `1.3`

#### Scenario: Linked by branch
- **WHEN** a PR titled `fix typo` has head branch `feat/search-ui`
- **THEN** it appears under `search-ui` with no task

#### Scenario: GitHub not enabled
- **WHEN** a workspace has no `github = true`
- **THEN** the overview shows changes with no PR column data, `gh` is never run, and the status reads `gh off`

### Requirement: Check and review results from comments
When `comments.ci` or `comments.review` patterns are configured, the overview SHALL scan each linked PR's issue comments and take the newest match of each pattern. `ci` yields `status`, `sha` and optional `mode`; `review` yields `verdict` and `sha`. A result SHALL be **current** when its `sha` equals or is a prefix of the PR's head sha, else **stale**. A stale result SHALL be displayed as stale and SHALL NOT by itself raise a waiting item. With no pattern configured, no comment result is shown. When `checks = true` and no `ci` pattern is set, the conclusion of the head commit's check runs SHALL be used instead.

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
The overview SHALL list, per workspace, items needing the human: a proposal awaiting approval (state `proposal` with no merged PR), a PR whose current review verdict matches `comments.owner_needed`, a PR whose current CI status is `FAIL`, and every session of the workspace in `needs-input` (with its message). Each item SHALL name its change and PR or session. Sessions in `needs-input` SHALL appear even when linked to no change.

#### Scenario: Owner approval needed
- **WHEN** a PR's current review verdict is `owner approval needed` and `owner_needed = 'owner approval'`
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

### Requirement: Sessions linked to changes
A session SHALL link to at most one change of its workspace, using the first rule that matches: its label equals the change name or starts with `<change>-`; its branch (a `garage/<label>` worktree branch counts as its label) equals the change name or contains it as a whole token, or equals the head branch of a PR linked to the change. Linked sessions SHALL show on the change's row with their status. The overview SHALL take sessions from the daemon's session list and SHALL NOT read tmux.

#### Scenario: Linked by label
- **WHEN** session `garage/kowboy/search-ui-ui` exists and change `search-ui` exists
- **THEN** the session shows on the `search-ui` row

#### Scenario: Linked by worktree branch
- **WHEN** a session's branch is `feat/billing-v2` and change `billing-v2` exists
- **THEN** the session shows on the `billing-v2` row

### Requirement: GitHub access is cached, rate-limit aware and offline-safe
GitHub data SHALL be read only through `gh api` REST. Responses SHALL be cached with their ETag under `~/.garage/cache/overview/` and revalidated with `If-None-Match`. The overview SHALL record `x-ratelimit-remaining`; below 200 the poll interval SHALL double, and at 0 no call SHALL be made until `x-ratelimit-reset`. A missing `gh`, a login failure, a network failure or a 15 second timeout SHALL serve the cached data and show one status (`gh off`, `gh not logged in`, `gh offline`, `rate limited until <time>`). None of these SHALL delay the wall or fail the daemon endpoint. Deleting the cache directory SHALL only cause a slower next refresh.

#### Scenario: Unchanged data costs nothing
- **WHEN** GitHub answers `304 Not Modified` to a revalidation
- **THEN** the cached body is used and the data is unchanged

#### Scenario: Rate limit exhausted
- **WHEN** a response reports `x-ratelimit-remaining: 0`
- **THEN** no further `gh` call is made before the reset time and the cached PRs are shown with `rate limited until <time>`

#### Scenario: Offline
- **WHEN** `gh` fails to reach github.com
- **THEN** cached PRs are shown with `gh offline` and local OpenSpec data is unaffected

### Requirement: Refresh runs only for a consumer
The overview SHALL refresh only while a consumer exists: an open overlay, a running `tasks` command, or an API client that requested the overview in the last 60 seconds. Local files SHALL be checked every 5 seconds, GitHub every 60 seconds, git fetch every 5 minutes. A manual refresh SHALL do all three at once. With no consumer, no `git` fetch and no `gh` call SHALL run.

#### Scenario: Idle costs nothing
- **WHEN** nobody has opened the overview or called the endpoint for 60 seconds
- **THEN** the daemon runs no overview fetch or `gh` call

#### Scenario: Edit shows within seconds
- **WHEN** the overlay is open and a `tasks.md` checkbox is ticked in an editor
- **THEN** the row's `done/total` updates within about 5 seconds

### Requirement: Wall overlay
The wall SHALL provide a read-only overlay opened from the garage layer by `o` (configurable per open question 5) showing the focused workspace's waiting list and changes, as in the design mockup. It SHALL read only the daemon API. `j`/`k` SHALL move the selection with wrap, `Enter` SHALL focus the selected row's session (switching workspace and view) or, with none, open its PR, `p` SHALL open the PR URL, `c` toggles per-task detail, `r` refreshes, `Tab` cycles workspaces, `Esc` closes. Amber SHALL be used only for items needing the human. No key SHALL change any repo, PR or session.

#### Scenario: Jump to the session
- **WHEN** the overlay is open, a change row with a linked session is selected and the user presses `Enter`
- **THEN** the overlay closes and the wall focuses that session, switching workspace and view if needed

#### Scenario: Open the PR
- **WHEN** a PR row is selected and the user presses `p`
- **THEN** the PR URL opens through the wall's link opener and the overlay stays open

#### Scenario: Empty workspace
- **WHEN** the workspace has no OpenSpec changes
- **THEN** the overlay says `no OpenSpec changes in kowboy` and lists any waiting sessions

### Requirement: Tasks command
`claude-garage tasks` SHALL print the overview as plain text, one block per workspace, with `--workspace <name>` to select one, `--json` for the API model, `--local` to skip fetch and GitHub, and `--all` to include unlinked PRs. It SHALL use the daemon when running and SHALL fall back to an in-process local-only collection when it is not, saying so. The word `tasks` SHALL be a reserved core command word for plugins. Exit code SHALL be 0 whenever the overview was produced, including with warnings.

#### Scenario: Daemon down
- **WHEN** `claude-garage tasks` runs while the daemon is not running
- **THEN** local changes are printed with the note `daemon not running: local data only` and the exit code is 0

#### Scenario: JSON for scripts
- **WHEN** `claude-garage tasks --json --workspace kowboy` runs
- **THEN** stdout is the JSON model for that workspace and nothing else

### Requirement: Read-only and local
The overview SHALL NOT approve, merge, comment on, label or close a PR, edit any repo file, start, stop or message a session, or open a network connection other than `git fetch` to the repo's own `origin` and `gh api` to github.com for workspaces with `github = true`. It SHALL send no telemetry.

#### Scenario: Working tree untouched
- **WHEN** an overview refresh runs against a repo with uncommitted edits
- **THEN** the files, index and branch are byte-identical afterwards

#### Scenario: No write calls to GitHub
- **WHEN** any overview refresh runs
- **THEN** every `gh api` call is a `GET`
