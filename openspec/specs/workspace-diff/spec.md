# workspace-diff

## Purpose

Read-only per-workspace git diff API: tracked + untracked changes, per-file stats, renames, binary handling, size budgets with explicit truncation.

## Requirements

### Requirement: Diff endpoint returns changed-file list with per-file detail
The daemon SHALL expose `GET /api/diff/:workspace` returning, for the workspace's registered directory, a list of changed files. Each entry SHALL include the file `path`, a `changeType` (one of `added`, `modified`, `deleted`, `renamed`, `untracked`), per-file line-change stats (`added`/`deleted` counts), and a `binary` flag. Renamed entries SHALL include both the old and new path.

#### Scenario: Mixed change types reported with stats
- **WHEN** workspace `kowboy`'s directory has one modified tracked file (+3/-1), one new untracked file, and one file renamed from `old.ts` to `new.ts`
- **THEN** `GET /api/diff/kowboy` returns three entries: the modified file with `changeType: "modified"` and stats `{added:3, deleted:1}`, the new file with `changeType: "untracked"`, and the renamed file with `changeType: "renamed"` including both `old.ts` and `new.ts`

#### Scenario: Binary file flagged instead of diffed as text
- **WHEN** a workspace has a modified binary file (e.g. a `.png`)
- **THEN** its entry in the changed-file list has `binary: true` and no line-change stats are fabricated for it

### Requirement: Diff content covers tracked changes and untracked files
The response SHALL include unified diff content that covers both changes to tracked files (staged or unstaged) and the full content of untracked files, so a client never has to make a second call to see what a new file contains.

#### Scenario: Untracked file content appears in the diff
- **WHEN** workspace `kowboy` has one new untracked file `notes.md` with content
- **THEN** `GET /api/diff/kowboy`'s diff content includes `notes.md`'s content rendered as an addition, not merely listed by name

#### Scenario: Tracked unstaged and staged changes both included
- **WHEN** workspace `kowboy` has one staged modification and one unstaged modification to different tracked files
- **THEN** `GET /api/diff/kowboy` includes unified diff content for both files

### Requirement: Diff response supports per-file hunk rendering
The response SHALL correlate the changed-file list with the diff content such that a client can extract and render each file's own hunks independently, without re-parsing the entire diff to guess file boundaries.

#### Scenario: Client can isolate one file's hunks
- **WHEN** the response for a workspace with three changed files is returned
- **THEN** for each entry in the changed-file list, the diff content contains a clearly delimited section (matching that entry's `path`) that a client can extract as that file's own hunks

### Requirement: Diff endpoint is strictly read-only
`GET /api/diff/:workspace` SHALL NOT mutate the repository or its index in any way — it SHALL NOT stage files, create `.git/index.lock`, change the working tree, or alter the status of any untracked file.

#### Scenario: Untracked file remains untracked after repeated calls
- **WHEN** `GET /api/diff/kowboy` is called twice in a row against a workspace with one untracked file
- **THEN** after both calls, `git status` for that directory still reports the file as untracked (not staged), and no `.git/index.lock` is left behind

#### Scenario: Working tree content is unchanged by the call
- **WHEN** `GET /api/diff/:workspace` is called against a workspace with a modified tracked file
- **THEN** the file's on-disk content and its staged/unstaged state are identical before and after the call

### Requirement: Unknown workspace returns 404
`GET /api/diff/:workspace` SHALL respond 404 when `:workspace` is not a registered workspace.

#### Scenario: Diff for unregistered workspace
- **WHEN** `GET /api/diff/ghost` is called and no workspace named `ghost` is registered
- **THEN** the daemon responds 404

### Requirement: Non-git registered directory returns empty file list
When a registered workspace's directory is not a git repository, `GET /api/diff/:workspace` SHALL respond 200 with an empty changed-file list and empty diff content, rather than an error.

#### Scenario: Plain directory with no .git
- **WHEN** workspace `scratch` is registered against a directory with no `.git`
- **THEN** `GET /api/diff/scratch` responds 200 with an empty changed-file list

### Requirement: Oversized diffs are truncated, not failed
When the generated diff content exceeds the daemon's internal size budget, the daemon SHALL truncate the diff content and set an explicit `truncated: true` flag on the response, rather than failing the request or silently omitting affected files from the changed-file list.

#### Scenario: Large diff truncated with flag set
- **WHEN** a workspace's diff content exceeds the daemon's size budget
- **THEN** `GET /api/diff/:workspace` still responds 200, the changed-file list still lists every changed file, the diff content is truncated, and `truncated` is `true`

#### Scenario: Diff within budget is not marked truncated
- **WHEN** a workspace's diff content is within the daemon's size budget
- **THEN** the response's `truncated` flag is `false` (or absent) and the diff content is complete
