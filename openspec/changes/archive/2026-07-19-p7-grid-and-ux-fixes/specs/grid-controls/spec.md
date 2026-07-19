# grid-controls

## ADDED Requirements

### Requirement: Balanced default grid layout
When a workspace has no persisted dockview layout (first view, corrupt snapshot, or reset), the default layout SHALL arrange the workspace's N sessions in a near-square grid — `ceil(sqrt(N))` columns, filling row-major in session order — instead of a full-width vertical stack. A persisted layout SHALL continue to load exactly as saved; only the zero-config default changes.

#### Scenario: Four sessions build a 2×2 grid
- **WHEN** workspace `kowboy` has four sessions and no persisted layout exists for it
- **THEN** the grid renders the four terminals as two columns × two rows, each cell of comparable size, rather than four stacked full-width rows

#### Scenario: Persisted custom layouts are untouched
- **WHEN** a workspace has a persisted layout previously arranged by the user (e.g. one wide cell over two narrow ones)
- **THEN** loading that workspace restores the persisted arrangement unchanged; the balanced-grid default applies only when no valid persisted layout exists

#### Scenario: Reset layout rebuilds the balanced grid
- **WHEN** the user activates the reset-layout control for a workspace with five sessions
- **THEN** the persisted layout is discarded and the grid rebuilds as the balanced near-square default (3 columns: 2+2+1), not a vertical stack

### Requirement: Joining sessions split the longest axis
When a session joins an existing layout (spawn, restore completion, reconcile after a layout load), its panel SHALL be added by splitting the largest existing panel along that panel's longer axis — `right` when the panel is wider than tall, `below` otherwise — so the grid stays balanced as sessions come and go.

#### Scenario: New session splits a wide panel to the right
- **WHEN** the largest panel in the focused workspace's grid is wider than it is tall and a new session is spawned
- **THEN** the new session's panel appears as a right-split of that panel (side by side), not stacked below it

#### Scenario: New session splits a tall panel below
- **WHEN** the largest panel is taller than it is wide and a new session joins
- **THEN** the new panel is added below that panel

### Requirement: Split controls
The grid SHALL provide explicit split controls in the style of VS Code's terminal: a split-right and a split-down control in the grid header acting on the focused cell, and the same pair on each cell's title bar. Activating a split control SHALL spawn a new session in the workspace (via the existing session-spawn API) and place its panel by splitting the target cell in the chosen direction. The `\` key SHALL split the focused cell right, subject to the same terminal-focus suppression rules as other chrome keybindings.

#### Scenario: Split right from the cell title bar
- **WHEN** the user activates the split-right control on session `checkout`'s cell
- **THEN** a new session is spawned in that workspace and its terminal appears side by side to the right of `checkout`'s cell, and it becomes the focused cell

#### Scenario: Backslash splits the focused cell
- **WHEN** no terminal has keyboard focus and the user presses `\`
- **THEN** the focused cell splits right with a newly spawned session, identically to the toolbar control

#### Scenario: Backslash typed into a terminal is not intercepted
- **WHEN** a terminal has keyboard focus and the user types `\`
- **THEN** the character is delivered to that terminal's pty and no split occurs

### Requirement: Maximize toggle
The grid SHALL provide a maximize control (grid-header toolbar) and an `m` keybinding that maximize the focused cell to fill the entire grid area, hiding the other cells without disturbing the underlying layout; activating it again SHALL restore the previous arrangement exactly. Focus changes that bring a different cell forward (e.g. the `a` jump) while maximized SHALL exit maximize rather than leave the user looking at the wrong cell.

#### Scenario: Maximize and restore round-trips the layout
- **WHEN** the user maximizes the focused cell in a four-cell custom arrangement and then activates maximize again
- **THEN** while maximized only that cell is visible filling the grid, and on restore all four cells reappear in exactly their prior arrangement

#### Scenario: Needs-input jump exits maximize
- **WHEN** cell `build` is maximized and the user presses `a` to jump to needs-input session `checkout`
- **THEN** maximize exits and `checkout`'s cell is visible and focused in the restored grid

### Requirement: Spawn menu in the grid header
The grid header SHALL provide a `+` control with a menu offering "new session" and "new worktree session" for the focused workspace. Choosing an item SHALL spawn the session (with `worktree: true` for the worktree item) and place it per the longest-axis rule relative to the focused cell. This complements, and does not replace, the rail's per-workspace new-session control.

#### Scenario: Spawning a worktree session from the grid header
- **WHEN** the user opens the grid header's `+` menu and chooses "new worktree session"
- **THEN** a session is spawned with `worktree: true` in the focused workspace and its terminal joins the grid beside the focused cell
