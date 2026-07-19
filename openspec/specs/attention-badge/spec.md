# attention-badge

## Purpose

Aggregate needs-input signalling: the header badge counting blocked sessions across all workspaces, mirrored into the page title.

## Requirements

### Requirement: Aggregate needs-input badge
The app header SHALL display a badge with the count of sessions in status `needs-input` across all workspaces, updating live from the same status channel as the rail. Activating the badge SHALL perform the same jump as the `a` keybinding (focus the workspace and terminal of a needs-input session, un-hiding it if hidden). When the count is zero the badge SHALL render in a visually quiet state (e.g. "all clear") rather than disappearing, so its location stays stable.

#### Scenario: Badge counts across workspaces and jumps on click
- **WHEN** workspace `kowboy` has one `needs-input` session and workspace `blog` has another, while `garage-dev` is focused
- **THEN** the badge shows a count of 2, and clicking it focuses a needs-input session exactly as pressing `a` would

#### Scenario: Zero state is quiet but present
- **WHEN** no session anywhere is in status `needs-input`
- **THEN** the badge renders in its quiet all-clear state in the same header position, without the attention styling

### Requirement: Page-title badge
The client SHALL mirror the needs-input count into `document.title` (e.g. `(2) claude-garage`) whenever the count is non-zero, and restore the plain title when it returns to zero, so a backgrounded browser tab is itself a glanceable status light.

#### Scenario: Title gains and loses the count
- **WHEN** a session transitions into `needs-input` while the tab is backgrounded, and the user later answers it
- **THEN** the tab title shows the parenthesized count while non-zero and reverts to the plain title once the count is zero
