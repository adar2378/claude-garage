## MODIFIED Requirements

### Requirement: Finish a worktree (merge, discard, or keep)
The daemon SHALL expose `POST /api/worktrees/finish` accepting `{id?, worktree:{path, branch, repoDir}, action}`, where `action` is one of `merge`, `discard`, or `keep`, taking the worktree record from the prior DELETE response rather than re-deriving it from session state (the session and its metadata are already gone by the time finish runs).

For `action: "merge"`, the daemon SHALL first refuse with 409 if the worktree has uncommitted changes. Otherwise it SHALL run `git -C <repoDir> merge --no-ff <branch>`. A non-zero exit SHALL cause the daemon to run `git -C <repoDir> merge --abort` (so `repoDir` is not left mid-merge), respond 409 with git's stderr output, and leave the worktree and branch untouched. A zero exit SHALL cause the daemon to remove the worktree (`git worktree remove <path>`) and delete the branch (`git branch -d <branch>`).

For `action: "discard"`, the daemon SHALL force-remove the worktree (`git worktree remove --force <path>`) and force-delete the branch (`git branch -D <branch>`), but ONLY when `branch` matches `garage/*`. A `branch` value that does not match `garage/*` SHALL be refused (the daemon SHALL NOT delete it), guarding against ever deleting a repo's non-garage branch.

For `action: "keep"`, the daemon SHALL perform no git operations and SHALL respond success, leaving the worktree and branch exactly as they were.

#### Scenario: Merge succeeds and cleans up
- **WHEN** `POST /api/worktrees/finish` is called with `{worktree:{path:"~/.garage/worktrees/kowboy/feature", branch:"garage/feature", repoDir:"<kowboy dir>"}, action:"merge"}` and the merge has no conflicts
- **THEN** the daemon merges `garage/feature` into the repo's current branch with `--no-ff`, then removes the worktree and deletes the `garage/feature` branch

#### Scenario: Merge conflict is aborted and surfaces the error
- **WHEN** `action:"merge"` is requested and `git merge --no-ff garage/feature` exits non-zero due to a conflict
- **THEN** the daemon runs `git merge --abort` in `repoDir`, responds 409 with git's stderr in the response body, and the worktree directory and `garage/feature` branch both remain exactly as they were; `git -C <repoDir> status` shows no merge in progress

#### Scenario: Dirty worktree refuses merge
- **WHEN** `action:"merge"` is requested and the worktree has uncommitted changes
- **THEN** the daemon responds 409 naming the uncommitted changes and runs no merge

#### Scenario: Discard force-removes worktree and branch
- **WHEN** `POST /api/worktrees/finish` is called with `{worktree:{path:"~/.garage/worktrees/kowboy/feature", branch:"garage/feature", repoDir:"<kowboy dir>"}, action:"discard"}`
- **THEN** the daemon force-removes the worktree directory and force-deletes the `garage/feature` branch

#### Scenario: Discard refuses to delete a non-garage branch name
- **WHEN** `POST /api/worktrees/finish` is called with `action:"discard"` and a `worktree.branch` value of `main` (not matching `garage/*`)
- **THEN** the daemon refuses the branch deletion and does not run `git branch -D main`

#### Scenario: Keep is a no-op
- **WHEN** `POST /api/worktrees/finish` is called with `action:"keep"` for a given worktree record
- **THEN** the daemon performs no git operations against the worktree or branch, and responds success

## ADDED Requirements

### Requirement: Worktree record names the merge target
The worktree record returned by `DELETE /api/sessions/*` (live and `?meta=1`) SHALL include `target`: the branch currently checked out in `repoDir` (`git -C <repoDir> rev-parse --abbrev-ref HEAD`), or `null` if it cannot be read. This field is informational; `/api/worktrees/finish` SHALL ignore it.

#### Scenario: Target branch included
- **WHEN** a worktree session on `garage/feature` is killed and `repoDir` has `main` checked out
- **THEN** the response's `worktree` record includes `target: "main"`
