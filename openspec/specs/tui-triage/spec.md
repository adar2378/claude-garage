# tui-triage Specification

## Purpose
TBD - created by archiving change p8-nocterm-tui. Update Purpose after archive.
## Requirements
### Requirement: Salience-first ordering
Rail ordering SHALL be stable and salience-sorted over workspace families. A family is the set of workspaces whose directories share a parent folder (compared case-insensitively, trailing slash ignored); a workspace with no known directory is its own family. Family members SHALL be adjacent in the rail, families ordered by their first member's registration (then discovery) order and members keeping that order within the family. A family containing a `needs-input` session SHALL sort before families without one, and within a family workspaces containing a `needs-input` session SHALL sort first. Within a workspace `needs-input` sessions sort first. The strip SHALL show a per-workspace amber dot on each workspace tab that contains a blocked session, and the total blocked count SHALL be visible whenever it is non-zero.

#### Scenario: Blocked family bubbles up
- **WHEN** workspace B (rail position 2), a sibling of workspace C (position 3), gains a needs-input session while workspace A (position 1, a different folder) has none
- **THEN** B renders first, C directly after it, then A, and other relative orders are unchanged

#### Scenario: Siblings are adjacent
- **WHEN** the registry lists `et-mobile-app`, `mobile-apps`, `et-backend` where `et-mobile-app` and `et-backend` share a parent folder
- **THEN** `et-mobile-app` and `et-backend` are adjacent in the rail, with `mobile-apps` after them

#### Scenario: Folder case does not split a family
- **WHEN** two workspaces live under `.../development/x` and `.../Development/x`
- **THEN** they belong to one family

### Requirement: The `a` jump lands engaged
Pressing `a` in the garage layer SHALL focus the longest-waiting `needs-input` session across all workspaces — switching workspace if needed, swapping the session into the grid if it was overflow — and SHALL immediately enter the `engaged` layer on that tile, so the next keystroke reaches the blocked agent. If no session is blocked, `a` SHALL be a no-op with a brief strip notice.

#### Scenario: Two keystrokes to answer
- **WHEN** a session in another workspace has been waiting on a permission prompt and the user presses `a` then `1`
- **THEN** the wall switches to that workspace, that tile is engaged, and the byte `1` reaches the agent's permission prompt

### Requirement: Triage queue overlay
Pressing `A` (or clicking the strip badge) SHALL open an overlay listing all `needs-input` sessions across workspaces, sorted by waiting time (longest first). Each row SHALL show the session identity, waiting duration, and the session's notification message from the daemon when available. `j`/`k` SHALL move the selection, `Enter` SHALL close the overlay and perform the `a`-jump behavior on the selected session, and `Esc` SHALL close the overlay.

#### Scenario: Question text shown
- **WHEN** the daemon reports a blocked session whose notification message is "Claude needs your permission to use Bash"
- **THEN** the queue row for that session shows that message text

#### Scenario: Jump from queue lands engaged
- **WHEN** the user selects the second row and presses Enter
- **THEN** the overlay closes, that session's tile is focused and engaged

### Requirement: Waiting time display
For `needs-input` sessions the TUI SHALL display elapsed waiting time (since the status transition, from the daemon's `since` field) in the rail, the tile title, and the queue, updating at least every 30 seconds.

#### Scenario: Waiting time visible
- **WHEN** a session has been needs-input for 4 minutes
- **THEN** the rail row and queue row show a 4-minute waiting duration

### Requirement: Off-screen escalation
When a session transitions to `needs-input`, the TUI SHALL ring the terminal bell (SSH-safe) and update the terminal title (OSC) to include the blocked count. The TUI SHALL report visibility to the daemon (`POST /api/ui/visibility`) so daemon-side macOS notifications stay suppressed while the TUI is running in a visible terminal, using the same contract as the web UI.

#### Scenario: Bell on new blocked session
- **WHEN** a working session transitions to needs-input while the user is in another tmux window
- **THEN** the terminal bell is emitted once for that transition and the title shows the blocked count

