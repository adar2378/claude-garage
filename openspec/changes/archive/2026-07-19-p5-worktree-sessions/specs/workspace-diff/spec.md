## MODIFIED Requirements

### Requirement: Diff endpoint returns changed-file list with per-file detail
The daemon SHALL expose `GET /api/diff/:workspace` returning, for the workspace's registered directory, a list of changed files. Each entry SHALL include the file `path`, a `changeType` (one of `added`, `modified`, `deleted`, `renamed`, `untracked`), per-file line-change stats (`added`/`deleted` counts), and a `binary` flag. Renamed entries SHALL include both the old and new path.

`GET /api/diff/:workspace` SHALL accept an optional `?sessionId=<id>` query parameter. When the named session is live and its cwd (its worktree path, if it has one, per the `worktree-sessions` capability) differs from the workspace's registered directory, the daemon SHALL diff that path instead of the registered directory. When `sessionId` is absent, unknown, or resolves to a cwd matching the registered directory, the endpoint's behavior SHALL be unchanged from diffing the registered directory directly.

#### Scenario: Mixed change types reported with stats
- **WHEN** workspace `kowboy`'s directory has one modified tracked file (+3/-1), one new untracked file, and one file renamed from `old.ts` to `new.ts`
- **THEN** `GET /api/diff/kowboy` returns three entries: the modified file with `changeType: "modified"` and stats `{added:3, deleted:1}`, the new file with `changeType: "untracked"`, and the renamed file with `changeType: "renamed"` including both `old.ts` and `new.ts`

#### Scenario: Binary file flagged instead of diffed as text
- **WHEN** a workspace has a modified binary file (e.g. a `.png`)
- **THEN** its entry in the changed-file list has `binary: true` and no line-change stats are fabricated for it

#### Scenario: sessionId override diffs the session's worktree instead of the registry directory
- **WHEN** session `garage/kowboy/feature` is a live worktree session whose cwd is `~/.garage/worktrees/kowboy/feature`, and `GET /api/diff/kowboy?sessionId=garage%2Fkowboy%2Ffeature` is called
- **THEN** the daemon returns the changed-file list and diff content for `~/.garage/worktrees/kowboy/feature`, not for `kowboy`'s registered directory

#### Scenario: Unknown or non-overriding sessionId leaves the registry directory as the diff target
- **WHEN** `GET /api/diff/kowboy?sessionId=garage%2Fkowboy%2Fcheckout` is called for a session whose cwd matches `kowboy`'s registered directory (a non-worktree session)
- **THEN** the daemon returns the same changed-file list and diff content it would return without the `sessionId` parameter
