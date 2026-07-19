## MODIFIED Requirements

### Requirement: Spawn a garage session
The daemon SHALL create sessions via `POST /api/sessions` with body `{workspace, label}`, resolving the project directory from the workspace registry entry for `workspace` (instead of accepting a raw `dir` in the request body), by running `tmux new-session -d -s "garage/<workspace>/<label>" -c <resolved-dir> claude`. `workspace` and `label` MUST match `[a-z0-9-]+`; invalid names SHALL be rejected with 400. If `workspace` is not a registered workspace, the daemon SHALL respond 404 and SHALL NOT create a tmux session. If `workspace` is registered but its registry directory no longer exists on disk at spawn time, the daemon SHALL respond 400 and SHALL NOT create a tmux session. A name collision with an existing tmux session SHALL be rejected with 409.

The body MAY additionally include an optional `worktree: true` flag. When set, the resolved directory MUST be a git repository — if it is not, the daemon SHALL respond 400 and SHALL NOT create a tmux session — and the session SHALL instead be spawned in a dedicated git worktree per the `worktree-sessions` capability (worktree creation, naming, and metadata persistence), with the tmux cwd set to the worktree's path rather than the registry directory.

#### Scenario: Successful spawn
- **WHEN** workspace `garage-dev` is registered with an existing directory, and `POST /api/sessions` is called with `{workspace:"garage-dev", label:"main"}`
- **THEN** the daemon responds 201 with the session id `garage/garage-dev/main`
- **THEN** `tmux ls` on the host shows a detached session named `garage/garage-dev/main` running in the directory registered for `garage-dev`

#### Scenario: Duplicate name rejected
- **WHEN** `POST /api/sessions` is called twice with the same registered workspace and label
- **THEN** the second call responds 409 and no second tmux session is created

#### Scenario: Invalid name rejected
- **WHEN** `POST /api/sessions` is called with a label containing `:`, `.`, `/`, uppercase, or spaces
- **THEN** the daemon responds 400 and no tmux session is created

#### Scenario: Unknown workspace rejected
- **WHEN** `POST /api/sessions` is called with `{workspace:"never-registered", label:"main"}` and no workspace named `never-registered` exists in the registry
- **THEN** the daemon responds 404 and no tmux session is created

#### Scenario: Registry directory no longer exists
- **WHEN** workspace `kowboy` is registered pointing at a directory that has since been deleted or moved, and `POST /api/sessions` is called with `{workspace:"kowboy", label:"main"}`
- **THEN** the daemon responds 400 and no tmux session is created

#### Scenario: Worktree flag rejected for a non-git directory
- **WHEN** workspace `scratch` is registered against a directory with no `.git`, and `POST /api/sessions` is called with `{workspace:"scratch", label:"main", worktree:true}`
- **THEN** the daemon responds 400 and no tmux session is created
