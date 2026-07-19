## ADDED Requirements

### Requirement: Focus-dimming setting, off by default
The UI SHALL provide a focus-dimming setting that is OFF by default. When OFF, no zone SHALL be dimmed regardless of which column is active.

#### Scenario: Focus dimming is off on first use
- **WHEN** a user opens the pit wall in a browser that has never set the focus-dimming setting
- **THEN** no zone in the rail, grid, or changes pane is dimmed, regardless of which column is active

#### Scenario: All zones render normally when the setting is off
- **WHEN** the focus-dimming setting is OFF and the grid shows three terminals, one of them in the active column
- **THEN** all three terminals render at full brightness, with no opacity reduction applied to any of them

### Requirement: Enabling focus dimming spotlights the active column
When the focus-dimming setting is ON, the UI SHALL render every zone in the currently active column (workspace rail, terminal grid, or changes pane) at full brightness, and visually dim every zone in the other two columns.

#### Scenario: An inactive column's terminal cells dim when the setting is on
- **WHEN** the focus-dimming setting is ON, the grid column is active, and the changes pane shows a diff
- **THEN** every cell in the grid renders at full brightness and the changes pane's content renders visually dimmed

#### Scenario: Every zone in the active column is bright, not just one
- **WHEN** the focus-dimming setting is ON and the rail column is active, showing workspaces `garage-dev` and `kowboy` with several sessions each
- **THEN** every session row across both workspaces in the rail renders at full brightness — not only the row of whichever session is nominally selected

#### Scenario: Switching the active column moves the spotlight
- **WHEN** the focus-dimming setting is ON, the grid column is active, and the user clicks inside the changes pane
- **THEN** the changes pane becomes the active column and renders at full brightness, while the rail and grid dim

### Requirement: Active-column tracking by last interaction
The UI SHALL track which column is active based on the user's last interaction with it: a pointer click inside a column, or a keybinding that unambiguously targets one column.

#### Scenario: A pointer click activates its column
- **WHEN** the focus-dimming setting is ON and the user clicks anywhere inside the workspace rail, including on a session row
- **THEN** the rail becomes the active column

#### Scenario: A grid-targeted keybinding activates the grid column
- **WHEN** the focus-dimming setting is ON and the user presses `[`, `]`, `a`, or a digit `1`-`9`
- **THEN** the terminal grid becomes the active column

#### Scenario: A pane-targeted keybinding activates the pane column
- **WHEN** the focus-dimming setting is ON, the changes pane is visible (not collapsed), and the user presses `Tab`, `j`, or `k`
- **THEN** the changes pane becomes the active column

### Requirement: Focus resolution order for dimming
When the focus-dimming setting is ON, the UI SHALL resolve which surface is "focused" (rendered at full brightness) using the following order, taking the first that applies: (1) the help overlay, if open — it covers the page, so no column dimming logic applies; (2) review mode, if open — same; (3) otherwise, the active column, as tracked by the user's last interaction (pointer click or keybinding).

#### Scenario: Help overlay takes priority when open
- **WHEN** the focus-dimming setting is ON and the help overlay is open
- **THEN** the help overlay is treated as the focused surface and no column in the three-column layout is singled out as active for dimming purposes

#### Scenario: Review mode takes priority over column dimming
- **WHEN** the focus-dimming setting is ON and review mode is open
- **THEN** review mode is treated as the focused surface and no column in the three-column layout is singled out as active for dimming purposes

#### Scenario: The active column wins when no overlay or review mode is open
- **WHEN** the focus-dimming setting is ON, neither the help overlay nor review mode is open, and the grid was the last column interacted with
- **THEN** the grid column is treated as the focused surface and renders at full brightness

### Requirement: Needs-input zones are never dimmed
Regardless of which column is active, when the focus-dimming setting is ON, a rail row or grid cell whose session has status `needs-input` SHALL NEVER be dimmed — it SHALL always render at full brightness, including its status glyph, even when its column is not the active one.

#### Scenario: A needs-input grid cell stays bright while the grid column is inactive
- **WHEN** the focus-dimming setting is ON, the rail column is active, and grid cell `build` has status `needs-input`
- **THEN** `build`'s cell renders at full brightness even though the grid is not the active column

#### Scenario: A needs-input rail row stays bright while the rail column is inactive
- **WHEN** the focus-dimming setting is ON, the grid column is active, and workspace `kowboy`'s rail row has a session with status `needs-input`
- **THEN** `kowboy`'s needs-input session row renders at full brightness, not dimmed, despite the rail not being the active column

#### Scenario: A session leaving needs-input becomes eligible for dimming again
- **WHEN** the focus-dimming setting is ON, a previously needs-input session transitions to `working`, and that session's column is not the active column
- **THEN** that session's rail row and grid cell become dimmed like any other zone in a non-active column

### Requirement: Toggling focus dimming takes effect immediately
Toggling the focus-dimming setting SHALL take effect immediately, without requiring a page reload.

#### Scenario: Turning dimming on immediately dims non-active columns
- **WHEN** the focus-dimming setting is currently OFF and the user turns it ON via the settings popover
- **THEN** every zone in the two non-active columns dims immediately (except needs-input zones), with no page reload

#### Scenario: Turning dimming off immediately restores full brightness
- **WHEN** the focus-dimming setting is currently ON and the user turns it OFF via the settings popover
- **THEN** all zones in all three columns immediately return to full brightness, with no page reload

### Requirement: Focus-dimming setting persists per browser
The focus-dimming setting SHALL persist per browser (surviving page reloads and new tabs in that browser) and SHALL NOT be stored as daemon state shared across browsers or machines.

#### Scenario: Setting survives a page reload
- **WHEN** the user turns focus dimming ON and reloads the page in the same browser
- **THEN** focus dimming is still ON after reload

#### Scenario: Setting is local to the browser, not the daemon
- **WHEN** the user turns focus dimming ON in one browser and then opens the pit wall in a different browser (or a different machine) pointed at the same daemon
- **THEN** the focus-dimming setting in the second browser is unaffected by the first browser's setting
