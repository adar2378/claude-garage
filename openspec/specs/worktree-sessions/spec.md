# worktree-sessions

## Purpose

Opt-in git-worktree isolation per session: spawn-time creation, restore-into-worktree, and the merge/discard/keep finish flow, guarded to garage/* branches.

## Requirements

### Requirement: Worktree creation at spawn
When `POST /api/sessions` is called with `worktree: true`, the daemon SHALL require the workspace's registered directory to be a git repository; if it is not, the daemon SHALL respond 400 and SHALL NOT create a tmux session or a worktree. On success, the daemon SHALL run `git worktree add ~/.garage/worktrees/<workspace>/<label> -b garage/<label>` rooted at the repo's current HEAD, and SHALL use the resulting worktree path as the tmux session's `-c` cwd, so `claude` starts inside the worktree rather than the registered workspace directory. A collision between the target worktree path and an existing directory under `~/.garage/worktrees/<workspace>/` SHALL be resolved by suffixing the label (`-2`, `-3`, …). A collision between `garage/<label>` and an existing branch in the repo SHALL be resolved the same way, independently of the path suffix.

#### Scenario: Successful worktree spawn
- **WHEN** workspace `kowboy` is registered against a git repository and `POST /api/sessions` is called with `{workspace:"kowboy", label:"feature", worktree:true}`
- **THEN** the daemon creates a worktree at `~/.garage/worktrees/kowboy/feature` on a new branch `garage/feature` branched from the repo's current HEAD, and the resulting tmux session's cwd is that worktree path

#### Scenario: Non-git workspace directory rejected
- **WHEN** workspace `scratch` is registered against a directory with no `.git`, and `POST /api/sessions` is called with `{workspace:"scratch", label:"main", worktree:true}`
- **THEN** the daemon responds 400, and no tmux session or worktree is created

#### Scenario: Worktree path collision suffixed
- **WHEN** `~/.garage/worktrees/kowboy/feature` already exists on disk and `POST /api/sessions` is called again with `{workspace:"kowboy", label:"feature", worktree:true}`
- **THEN** the daemon creates the new worktree at `~/.garage/worktrees/kowboy/feature-2` instead of failing or overwriting the existing directory

#### Scenario: Branch name collision suffixed
- **WHEN** branch `garage/feature` already exists in workspace `kowboy`'s repo and a worktree session is spawned with label `feature`
- **THEN** the daemon creates the worktree on branch `garage/feature-2` instead of failing or reusing the existing branch

### Requirement: Worktree metadata persisted with the session's resume record
The daemon SHALL write a `worktree` object `{path, branch, repoDir}` into the session's resume metadata entry in `~/.garage/state.json` at spawn time, alongside the existing `sessionId`/workspace/label fields defined by session-restore. This write SHALL happen as part of spawn itself, not deferred to the status poller.

#### Scenario: Worktree metadata recorded at spawn
- **WHEN** `POST /api/sessions` is called with `{workspace:"kowboy", label:"feature", worktree:true}` and succeeds
- **THEN** `~/.garage/state.json`'s `sessions` map entry for `garage/kowboy/feature` includes a `worktree` object with `path` set to the created worktree's path, `branch` set to `garage/feature`, and `repoDir` set to `kowboy`'s registered directory

### Requirement: Restore recreates a worktree session in its worktree path
When `POST /api/sessions/restore` restores a session whose resume metadata includes a `worktree` object, the daemon SHALL use `worktree.path` as the tmux session's cwd instead of the workspace's registered directory. If `worktree.path` no longer exists on disk, the daemon SHALL respond 409, SHALL NOT create a tmux session, and SHALL retain the session's resume metadata (including the `worktree` object) unchanged so a subsequent finish (e.g. `discard`) can still target the recorded paths.

#### Scenario: Restoring a worktree session recreates it in the worktree
- **WHEN** `POST /api/sessions/restore` is called with `{id:"garage/kowboy/feature"}` for a restorable session whose metadata records `worktree.path` `~/.garage/worktrees/kowboy/feature` (still present on disk) and `sessionId` `abc123`
- **THEN** the daemon runs `tmux new-session -d -s "garage/kowboy/feature" -c ~/.garage/worktrees/kowboy/feature claude --resume abc123`, not the registered workspace directory

#### Scenario: Restore fails when the worktree path is missing
- **WHEN** restore is attempted for a session whose recorded `worktree.path` has been deleted from disk (e.g. manually removed outside the daemon)
- **THEN** the daemon responds 409, no tmux session is created, and the session's resume metadata (including the `worktree` object) remains in `state.json`

### Requirement: Kill response includes the worktree record
`DELETE /api/sessions/:id` for a session with worktree metadata SHALL, after killing the tmux session and removing the session's resume metadata per existing session-restore semantics, include the worktree record in the response body as `{deleted:true, worktree:{path, branch, repoDir}}`, so a client can offer the finish flow after the session is gone. For a session without worktree metadata, the response SHALL omit the `worktree` field.

#### Scenario: Killing a worktree session returns its worktree record
- **WHEN** `DELETE /api/sessions/garage%2Fkowboy%2Ffeature` is called for a live worktree session
- **THEN** the daemon responds with `deleted:true` and a `worktree` object containing that session's recorded `path`, `branch`, and `repoDir`, in addition to killing the tmux session and removing its resume metadata

#### Scenario: Killing a non-worktree session omits the worktree field
- **WHEN** `DELETE /api/sessions/garage%2Fkowboy%2Fcheckout` is called for a live session that was not spawned with `worktree:true`
- **THEN** the daemon responds with `deleted:true` and no `worktree` field

### Requirement: Finish a worktree (merge, discard, or keep)
The daemon SHALL expose `POST /api/worktrees/finish` accepting `{id?, worktree:{path, branch, repoDir}, action}`, where `action` is one of `merge`, `discard`, or `keep`, taking the worktree record from the prior DELETE response rather than re-deriving it from session state (the session and its metadata are already gone by the time finish runs).

For `action: "merge"`, the daemon SHALL run `git -C <repoDir> merge --no-ff <branch>`. A non-zero exit SHALL cause the daemon to respond 409 with git's stderr output, and SHALL leave the worktree and branch untouched. A zero exit SHALL cause the daemon to remove the worktree (`git worktree remove <path>`) and delete the branch (`git branch -d <branch>`).

For `action: "discard"`, the daemon SHALL force-remove the worktree (`git worktree remove --force <path>`) and force-delete the branch (`git branch -D <branch>`), but ONLY when `branch` matches `garage/*`. A `branch` value that does not match `garage/*` SHALL be refused (the daemon SHALL NOT delete it), guarding against ever deleting a repo's non-garage branch.

For `action: "keep"`, the daemon SHALL perform no git operations and SHALL respond success, leaving the worktree and branch exactly as they were.

#### Scenario: Merge succeeds and cleans up
- **WHEN** `POST /api/worktrees/finish` is called with `{worktree:{path:"~/.garage/worktrees/kowboy/feature", branch:"garage/feature", repoDir:"<kowboy dir>"}, action:"merge"}` and the merge has no conflicts
- **THEN** the daemon merges `garage/feature` into the repo's current branch with `--no-ff`, then removes the worktree and deletes the `garage/feature` branch

#### Scenario: Merge conflict keeps everything and surfaces the error
- **WHEN** `action:"merge"` is requested and `git merge --no-ff garage/feature` exits non-zero due to a conflict
- **THEN** the daemon responds 409 with git's stderr in the response body, and the worktree directory and `garage/feature` branch both remain exactly as they were

#### Scenario: Discard force-removes worktree and branch
- **WHEN** `POST /api/worktrees/finish` is called with `{worktree:{path:"~/.garage/worktrees/kowboy/feature", branch:"garage/feature", repoDir:"<kowboy dir>"}, action:"discard"}`
- **THEN** the daemon force-removes the worktree directory and force-deletes the `garage/feature` branch

#### Scenario: Discard refuses to delete a non-garage branch name
- **WHEN** `POST /api/worktrees/finish` is called with `action:"discard"` and a `worktree.branch` value of `main` (not matching `garage/*`)
- **THEN** the daemon refuses the branch deletion and does not run `git branch -D main`

#### Scenario: Keep is a no-op
- **WHEN** `POST /api/worktrees/finish` is called with `action:"keep"` for a given worktree record
- **THEN** the daemon performs no git operations against the worktree or branch, and responds success
