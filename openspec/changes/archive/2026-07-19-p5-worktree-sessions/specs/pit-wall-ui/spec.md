## MODIFIED Requirements

### Requirement: New-session affordance per workspace
The UI SHALL provide a per-workspace control to spawn a new session in that workspace, which calls the session-spawn API (resolving the directory via the workspace registry) and adds the resulting session to the grid.

The new-session control SHALL include a worktree toggle. When enabled, the spawn call SHALL include `worktree: true` per the `worktree-sessions` capability. The toggle's last-used state SHALL be remembered per workspace (e.g. via `localStorage`, keyed by workspace) and SHALL be used to pre-set the toggle the next time the new-session control is opened for that same workspace.

#### Scenario: Spawning a session from the rail
- **WHEN** the user activates the new-session control for workspace `kowboy` and supplies a label
- **THEN** the daemon spawns a session `garage/kowboy/<label>` via the registry-resolved directory, and it appears in the rail and (if `kowboy` is focused) in the terminal grid

#### Scenario: Worktree toggle state is remembered per workspace
- **WHEN** the user enables the worktree toggle while spawning a session for workspace `kowboy`, and later reopens the new-session control for `kowboy`
- **THEN** the worktree toggle is pre-set to enabled; opening the new-session control for a different workspace `garage-dev` that has no remembered state does not inherit `kowboy`'s toggle state

## ADDED Requirements

### Requirement: Worktree finish prompt
For a session that was spawned with a worktree (per the `worktree-sessions` capability), the close flow's second step (after the two-step ✕ kill confirm) SHALL offer an inline `merge` / `discard` / `keep` choice instead of simply completing. Each action SHALL be reachable with a single click (no modal). Any error returned by the corresponding `POST /api/worktrees/finish` call (e.g. a merge conflict) SHALL be surfaced inline in that same prompt, and the prompt SHALL remain available for the user to retry a different action. Sessions that were not spawned with a worktree SHALL keep the existing two-step ✕ confirm behavior unchanged, with no finish prompt shown.

#### Scenario: Finish prompt appears after killing a worktree session
- **WHEN** the user completes the two-step ✕ confirm for a worktree session `garage/kowboy/feature`
- **THEN** the session is killed and an inline prompt appears offering `merge`, `discard`, and `keep` for that session's worktree

#### Scenario: Merge conflict surfaces inline and keeps the worktree
- **WHEN** the user selects `merge` in the finish prompt and the daemon responds 409 with git's conflict message
- **THEN** the prompt displays git's error message inline, and the worktree and branch remain on disk (nothing is silently discarded)

#### Scenario: Non-worktree sessions keep the existing close behavior
- **WHEN** the user completes the two-step ✕ confirm for a session that was not spawned with `worktree:true`
- **THEN** the session is killed and no finish prompt is shown, matching existing (pre-p5) behavior
