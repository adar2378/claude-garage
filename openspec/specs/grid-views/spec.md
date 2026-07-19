# grid-views

## Purpose

Named views per workspace — each view its own arrangement of 1..n sessions, one view on screen at a time — so a session can stand alone while others stay grouped, without ever weakening needs-input triage.

## Requirements

### Requirement: Detach a session into its own view
Each live cell SHALL offer a detach control that moves the session into its own standalone view (named after its label): the grid then shows only the remaining sessions of the current view, and selecting the detached session shows it alone filling the grid area. A detached cell's control SHALL become a rejoin affordance returning the session to the main view. Views are derived client state (assignments in localStorage keyed per workspace); a view whose last member ends SHALL simply cease to exist.

#### Scenario: Detach and switch round-trip
- **WHEN** workspace `kowboy` shows sessions `a`, `b`, `c` and the user detaches `c`
- **THEN** the grid shows only `a` and `b`; selecting `c` (rail row or view strip) shows `c` alone; `c`'s rejoin control returns all three to one grid

### Requirement: View strip and rail tier appear only with multiple views
With two or more views, the grid SHALL show a view strip (one tab per view: name, session count, and an accent dot when any member is `needs-input`), and the rail SHALL group the workspace's session rows under clickable view-header rows carrying the same aggregated signal. With a single view, both SHALL be absent — the workspace renders exactly as before views existed.

#### Scenario: Single-view workspaces are unchanged
- **WHEN** a workspace has no detached sessions
- **THEN** no view strip and no rail view tier render for it

#### Scenario: Background view carries the attention dot
- **WHEN** a session in a non-focused view transitions to `needs-input`
- **THEN** that view's strip tab and rail header show the accent dot while the current view stays on screen

### Requirement: Focus always switches to the focused session's view
Whenever the focused session belongs to a view other than the one on screen — rail click, the `a` jump, or terminal cycling — the grid SHALL switch to that session's view. No view state can leave a blocked session unreachable or invisible after a jump.

#### Scenario: `a` jumps across views
- **WHEN** the standalone view `c` is on screen and session `a` (in the main view) enters `needs-input`
- **THEN** pressing `a` switches the grid to the main view with session `a` focused

### Requirement: Per-view layouts persist
Each view SHALL persist its own dockview arrangement (the main view keeps the workspace's pre-views layout key for backward compatibility), and the focused view SHALL persist per workspace, both restored on reload. Sessions spawned by an explicit split while a detached view is focused SHALL join that view.

#### Scenario: Focused view survives reload
- **WHEN** the user detaches `c`, leaves its view focused, and reloads
- **THEN** the grid renders `c` standalone again, and switching to main restores the `a`+`b` arrangement
