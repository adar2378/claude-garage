## MODIFIED Requirements

### Requirement: New-session affordance per workspace
The UI SHALL provide a per-workspace control to spawn a new session in that workspace, which calls the session-spawn API (resolving the directory via the workspace registry) and adds the resulting session to the grid.

The UI SHALL also provide a control to add a new workspace to the registry. Activating this control SHALL, by default, invoke the native folder picker (`POST /api/pick-directory`) rather than immediately prompting the user to type an absolute path. A manual path-entry fallback SHALL remain reachable from the same flow for cases where the picker is unavailable, is cancelled, or does not apply (e.g. a non-macOS daemon or an SSH-forwarded browser). Picker behavior, name derivation from the chosen folder, and the registration endpoint itself are specified by the `workspace-picker` capability.

#### Scenario: Spawning a session from the rail
- **WHEN** the user activates the new-session control for workspace `kowboy` and supplies a label
- **THEN** the daemon spawns a session `garage/kowboy/<label>` via the registry-resolved directory, and it appears in the rail and (if `kowboy` is focused) in the terminal grid

#### Scenario: Add-workspace control opens the native picker by default
- **WHEN** the user activates the add-workspace control
- **THEN** the UI invokes the native folder picker rather than immediately prompting for a manually typed path

#### Scenario: Manual path entry remains reachable as a fallback
- **WHEN** the user activates the add-workspace control
- **THEN** a manual path-entry fallback (e.g. a "type a path instead" link) is reachable for adding a workspace without going through the native picker

### Requirement: Pit wall keybindings
The UI SHALL support the following global keybindings: `1`–`9` switch the focused workspace to the Nth workspace in the rail's flattened rendered order; `[` and `]` cycle the focused terminal to the previous/next session within the current workspace's grid; `a` jumps focus (workspace and terminal) to a session with status `needs-input`, searching across all workspaces, not only the currently focused one.

The rail's flattened rendered order is the order workspaces visually appear top-to-bottom, including nested (indented) workspaces at their rendered position — nesting depth does not exempt a workspace from being indexed by `1`–`9`.

#### Scenario: Number key switches workspace
- **WHEN** the rail shows `kowboy` as the 2nd workspace and the user presses `2`
- **THEN** the focused workspace becomes `kowboy` and its grid renders

#### Scenario: Number key indexes into the flattened rendered order including nested workspaces
- **WHEN** the rail renders top-level workspace `garage` as the 1st rendered row and its nested workspace `garage-ui` (indented beneath it, by directory containment) as the 2nd rendered row, and the user presses `2`
- **THEN** the focused workspace becomes `garage-ui`

#### Scenario: Bracket keys cycle terminal focus within the workspace
- **WHEN** the focused workspace's grid has sessions `checkout` (focused) and `build`, and the user presses `]`
- **THEN** focus moves to `build`'s terminal

#### Scenario: `a` jumps to a blocked session in another workspace
- **WHEN** the focused workspace is `garage-dev` (no `needs-input` sessions) and workspace `kowboy` has a session `checkout` with status `needs-input`
- **THEN** pressing `a` switches the focused workspace to `kowboy` and focuses `checkout`'s terminal
