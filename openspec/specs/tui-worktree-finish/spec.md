# tui-worktree-finish Specification

## Purpose
After closing a worktree session, the TUI asks whether to merge, discard or keep its branch and worktree.
## Requirements
### Requirement: Finish overlay after closing a worktree session
When the TUI closes a session (`x x`) and the daemon's DELETE response carries a worktree record `{path, branch, repoDir}`, the TUI SHALL open a finish overlay that holds that record. This SHALL apply both to live sessions and to restorable sessions closed through the `?meta=1` path. The overlay SHALL name the branch and the branch currently checked out in `repoDir` (the merge target), and SHALL offer: `m` merge, `d` discard (armed: a second `d` confirms), and `k` or `Esc` keep. Keep SHALL close the overlay without any daemon call.

#### Scenario: Overlay opens after closing a worktree session
- **WHEN** the user presses `x x` on a worktree session on branch `garage/feature` in a repo whose checked-out branch is `main`
- **THEN** the session is closed and an overlay opens naming `garage/feature` and `main`, with merge, discard and keep choices

#### Scenario: Closing a plain session opens no overlay
- **WHEN** the user presses `x x` on a non-worktree session
- **THEN** the session is closed and no finish overlay opens

#### Scenario: Restorable worktree session also gets the overlay
- **WHEN** the user closes a restorable (non-live) worktree session
- **THEN** the same finish overlay opens with the record from the `?meta=1` DELETE response

#### Scenario: Keep leaves everything in place
- **WHEN** the overlay is open and the user presses `k` or `Esc`
- **THEN** the overlay closes, no request is sent to `/api/worktrees/finish`, and the worktree and branch remain

### Requirement: Merge and discard call the finish endpoint
Choosing merge SHALL send `POST /api/worktrees/finish` with `action: "merge"` and the held record; confirming discard SHALL send `action: "discard"`. While a request is in flight the overlay SHALL ignore further choices. On success the overlay SHALL close and the strip SHALL show a notice naming the branch and the outcome. On an error response (e.g. 409 dirty worktree or merge conflict) the overlay SHALL stay open and show the daemon's error message inline, so the user can retry or keep.

#### Scenario: Merge succeeds
- **WHEN** the user presses `m` and the daemon responds success
- **THEN** the overlay closes and the strip shows that `garage/feature` was merged into `main`

#### Scenario: Discard needs confirmation
- **WHEN** the user presses `d` once
- **THEN** the overlay shows a "press d again to discard" prompt and no request is sent until the second `d`

#### Scenario: Dirty worktree error is shown inline
- **WHEN** the user presses `m` and the daemon responds 409 "worktree has uncommitted changes"
- **THEN** the overlay stays open, shows that message, and still offers merge, discard and keep

