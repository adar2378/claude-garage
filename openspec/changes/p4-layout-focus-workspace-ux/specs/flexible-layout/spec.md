## ADDED Requirements

### Requirement: Pop out a session into a separate browser window
The UI SHALL provide a per-cell control that opens that session's terminal in a separate browser window, rendering only that session's terminal (a single full-viewport terminal with a minimal header) and no other UI chrome from the main grid.

#### Scenario: Popping out a session opens a dedicated window
- **WHEN** the user activates the pop-out control on the grid cell for session `garage/kowboy/checkout`
- **THEN** a new browser window opens rendering only `checkout`'s live, interactive terminal, connected via `WS /term/:id`

#### Scenario: Popped-out terminal is fully interactive
- **WHEN** the popped-out window for `checkout` is open
- **THEN** the user can type into and observe live output from `checkout`'s terminal in that window exactly as they could in the main grid

### Requirement: Main grid shows a placeholder while a session is popped out
While a session's terminal is displayed in a popped-out window, the corresponding cell in the main terminal grid SHALL NOT render a live terminal for that session. Instead it SHALL render a placeholder indicating the session is being viewed in a separate window, together with a reclaim control.

#### Scenario: Grid cell becomes a placeholder on pop-out
- **WHEN** session `checkout` is popped out into a separate window
- **THEN** the main grid's cell for `checkout` shows a "viewing in separate window" placeholder instead of a live terminal, and does not open a second concurrent `WS /term/:id` connection for `checkout`

#### Scenario: Placeholder offers a reclaim control
- **WHEN** the main grid's cell for a popped-out session is showing the placeholder
- **THEN** the placeholder includes a reclaim control that the user can activate to return the session to the main grid

### Requirement: A popped-out session returns to the main grid when its window closes or its heartbeat goes stale
The popped-out window SHALL emit a heartbeat (at an interval no greater than 5 seconds) while open, and SHALL clear its heartbeat record when it closes cleanly. The main grid SHALL reclaim the cell — replacing the placeholder with a live terminal — either when the popout closes cleanly or when its heartbeat has not been renewed for more than 15 seconds (covering a force-killed or crashed window).

#### Scenario: Closing the popout window reclaims the cell
- **WHEN** the user closes the popped-out window for session `checkout` (e.g. via its close button, triggering a clean unload)
- **THEN** the main grid's cell for `checkout` promptly stops showing the placeholder and renders `checkout` as a live, interactive terminal again

#### Scenario: Activating the reclaim control returns the session without closing the window
- **WHEN** the user activates the reclaim control on the placeholder for a popped-out session while its window is still open
- **THEN** the main grid's cell renders that session as a live terminal again

#### Scenario: A crashed popout window is reclaimed after its heartbeat goes stale
- **WHEN** a popped-out window's heartbeat has not been renewed for more than 15 seconds (e.g. the window or its process was force-killed without a clean unload)
- **THEN** the main grid treats the session as no longer popped out and renders it as a live terminal in its cell

#### Scenario: A live heartbeat prevents premature reclaim
- **WHEN** a popped-out window's heartbeat has been renewed within the last 15 seconds
- **THEN** the main grid's cell for that session continues to show the placeholder, not a live terminal

### Requirement: Cells can be dragged and dropped to create splits
The terminal grid SHALL support dragging a cell and dropping it above, below, left, or right of another cell, creating a new split in the corresponding direction. Dropping a cell in this way SHALL rearrange the layout so both cells are visible in the resulting split arrangement.

#### Scenario: Dropping a cell to the right creates a vertical split
- **WHEN** the user drags cell `build`'s terminal and drops it on the right edge of cell `checkout`'s terminal
- **THEN** the layout rearranges so `checkout` and `build` are shown side by side, with `build` to the right of `checkout`

#### Scenario: Dropping a cell below creates a horizontal split
- **WHEN** the user drags cell `build`'s terminal and drops it on the bottom edge of cell `checkout`'s terminal
- **THEN** the layout rearranges so `checkout` and `build` are stacked, with `build` below `checkout`

### Requirement: Splitters resize adjacent panels
Every split created by drag-docking SHALL expose a draggable splitter between the adjacent panels that lets the user resize their relative proportions.

#### Scenario: Dragging a splitter resizes panels
- **WHEN** the user drags the splitter between `checkout`'s and `build`'s panels
- **THEN** the relative width or height allotted to each panel changes accordingly, and both terminals remain live and interactive at their new sizes

### Requirement: Layout persists per workspace across reloads
The terminal grid's layout (the drag-dock arrangement and split proportions) SHALL persist per workspace across page reloads, such that reloading the page and re-focusing a workspace restores the layout it had before the reload.

#### Scenario: Reloading the page preserves a custom layout
- **WHEN** the user arranges workspace `kowboy`'s grid into a custom split layout and then reloads the page
- **THEN** after reload, focusing `kowboy` again shows the same custom split layout, not the default stack

#### Scenario: Different workspaces keep independent layouts
- **WHEN** workspace `kowboy` has a custom split layout and workspace `garage-dev` has never been rearranged
- **THEN** focusing `garage-dev` shows its own (default) layout, unaffected by `kowboy`'s custom layout

### Requirement: A new session automatically joins the layout
When a new session is spawned into a workspace whose grid already has a persisted layout, the terminal grid SHALL automatically incorporate the new session's cell into that layout without requiring the user to manually place it.

#### Scenario: A newly spawned session appears in the existing layout
- **WHEN** workspace `kowboy` has a custom split layout with sessions `checkout` and `build`, and a new session `garage/kowboy/lint` is spawned
- **THEN** the grid renders `lint`'s live terminal alongside `checkout` and `build` within the layout, without discarding the existing arrangement

### Requirement: A dead session's panel is removed from the layout
When a session that has a panel in a workspace's layout is no longer present (e.g. it ends and is not restorable, or is otherwise removed from the workspace's session set), the terminal grid SHALL remove that session's panel from the layout, leaving the remaining panels in place.

#### Scenario: Ending a session removes its panel
- **WHEN** workspace `kowboy`'s layout includes panels for `checkout` and `build`, and `build`'s session ends and is removed from the workspace's session set
- **THEN** the grid's layout for `kowboy` no longer shows a panel for `build`, and `checkout`'s panel remains

### Requirement: Reset-layout control restores the default stack
The workspace header SHALL provide a reset-layout control that discards the workspace's persisted custom layout and restores the default vertical stack of all the workspace's current sessions.

#### Scenario: Reset layout restores the default stack
- **WHEN** workspace `kowboy` has a custom split layout and the user activates the reset-layout control
- **THEN** `kowboy`'s grid layout reverts to the default vertical stack of its current sessions, and this reverted layout is what persists across a subsequent reload
