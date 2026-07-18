# pit-wall-ui

## Purpose

The pit wall: workspace rail with needs-you-first ordering, simultaneous multi-terminal grid for the focused workspace, and chrome keybindings that coexist with typing into live terminals.

## Requirements

### Requirement: Workspace rail with nested sessions and status glyphs
The UI SHALL render a workspace rail listing every registered workspace, with that workspace's sessions nested beneath it. Each session SHALL display a status glyph corresponding to its status: `●` for `needs-input`, `◐` for `working`, `✓` for `done`, `○` for `idle`.

#### Scenario: Rail shows workspaces and nested sessions
- **WHEN** workspaces `kowboy` (sessions `checkout` status `needs-input`, `build` status `working`) and `garage-dev` (session `main` status `idle`) exist
- **THEN** the workspace rail shows both workspace names, each with its sessions nested underneath, and `checkout` renders `●`, `build` renders `◐`, `main` renders `○`

#### Scenario: Status glyph updates live
- **WHEN** a rendered session's status changes from `working` to `done` via the status push channel
- **THEN** the rail updates that session's glyph from `◐` to `✓` without a page reload

### Requirement: Needs-you-first ordering
The workspace rail SHALL order workspaces so that any workspace containing at least one session with status `needs-input` appears above workspaces with no such session. Within a workspace's session list, sessions with status `needs-input` SHALL be listed before sessions with any other status.

#### Scenario: Blocked workspace bubbles to the top
- **WHEN** workspace `alpha` (all sessions `idle`) was registered before workspace `beta` (one session `needs-input`)
- **THEN** the rail lists `beta` above `alpha`, despite `alpha` having been registered first

#### Scenario: Blocked session listed first within its workspace
- **WHEN** workspace `kowboy` has sessions `build` (`working`) and `checkout` (`needs-input`), with `build` having been created first
- **THEN** `checkout` is listed before `build` within the `kowboy` group

### Requirement: Simultaneous multi-terminal grid for the focused workspace
The terminal grid SHALL render every session belonging to the currently focused workspace as its own live, interactive xterm.js terminal, connected concurrently via `WS /term/:id`, stacked in the grid at the same time. A user SHALL be able to observe output streaming in more than one of these terminals simultaneously and interact with any of them, without switching windows, tabs, or navigating away from the grid.

#### Scenario: Two terminals of one workspace stream simultaneously with zero window switching
- **WHEN** the focused workspace `kowboy` has two sessions `checkout` and `build`, both actively producing output (e.g. Claude Code streaming a response in each)
- **THEN** both `checkout`'s and `build`'s terminals are visibly rendering and updating with their respective live output on screen at the same time, in the same view, with no window, tab, or route change required to see either

#### Scenario: Interacting with one terminal does not disturb the other
- **WHEN** both terminals of the focused workspace are visible and the user types into `checkout`'s terminal
- **THEN** the keystrokes are sent only to `checkout`'s pty and `build`'s terminal continues rendering its own independent output unaffected

#### Scenario: Grid updates when focus moves to a different workspace
- **WHEN** the user switches focus from workspace `kowboy` to workspace `garage-dev`
- **THEN** the grid now renders all of `garage-dev`'s sessions as live terminals, and `kowboy`'s terminal connections are no longer displayed in the grid

### Requirement: Focused-terminal highlight
Exactly one terminal within the grid SHALL be marked as focused at a time, visually distinguished (e.g. highlighted border) from the other terminals in the grid. Keyboard input not addressed to a specific terminal (e.g. typed characters) SHALL route to the focused terminal.

#### Scenario: Focused terminal is visually distinct
- **WHEN** the grid shows three terminals for the focused workspace
- **THEN** exactly one of the three has the focused-highlight styling applied, and the other two do not

#### Scenario: Clicking a terminal focuses it
- **WHEN** the user clicks on a non-focused terminal in the grid
- **THEN** that terminal becomes the focused terminal (gains the highlight) and the previously focused terminal loses it

### Requirement: Pit wall keybindings
The UI SHALL support the following global keybindings: `1`–`9` switch the focused workspace to the Nth workspace in rail order; `[` and `]` cycle the focused terminal to the previous/next session within the current workspace's grid; `a` jumps focus (workspace and terminal) to a session with status `needs-input`, searching across all workspaces, not only the currently focused one.

#### Scenario: Number key switches workspace
- **WHEN** the rail shows `kowboy` as the 2nd workspace and the user presses `2`
- **THEN** the focused workspace becomes `kowboy` and its grid renders

#### Scenario: Bracket keys cycle terminal focus within the workspace
- **WHEN** the focused workspace's grid has sessions `checkout` (focused) and `build`, and the user presses `]`
- **THEN** focus moves to `build`'s terminal

#### Scenario: `a` jumps to a blocked session in another workspace
- **WHEN** the focused workspace is `garage-dev` (no `needs-input` sessions) and workspace `kowboy` has a session `checkout` with status `needs-input`
- **THEN** pressing `a` switches the focused workspace to `kowboy` and focuses `checkout`'s terminal

### Requirement: Keybindings suppressed while typing into a terminal
Global pit-wall keybindings (`1`–`9`, `[`, `]`, `a`) SHALL NOT fire while keyboard focus is inside a terminal's input (i.e. the user is typing to send input to the running Claude Code process). Keystrokes in that state SHALL be delivered to the terminal's pty instead.

#### Scenario: Digit typed into terminal does not switch workspace
- **WHEN** a terminal has input focus and the user types `2` as part of a prompt to Claude
- **THEN** the `2` is sent to that terminal's pty as input, and the focused workspace does not change

#### Scenario: Bracket typed into terminal does not cycle focus
- **WHEN** a terminal has input focus and the user types `[` as part of program input
- **THEN** the `[` is sent to the pty and the focused terminal within the grid does not change

### Requirement: New-session affordance per workspace
The UI SHALL provide a per-workspace control to spawn a new session in that workspace, which calls the session-spawn API (resolving the directory via the workspace registry) and adds the resulting session to the grid.

#### Scenario: Spawning a session from the rail
- **WHEN** the user activates the new-session control for workspace `kowboy` and supplies a label
- **THEN** the daemon spawns a session `garage/kowboy/<label>` via the registry-resolved directory, and it appears in the rail and (if `kowboy` is focused) in the terminal grid
