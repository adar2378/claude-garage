# tui-actions Specification

## Purpose
TBD - created by archiving change p22-tui-seams. Update Purpose after archive.

## Requirements

### Requirement: Named actions
Every wall behaviour reachable from a key or a mouse click SHALL be a named action with a stable dotted id (for example `session.close`, `session.restart`, `session.focus`, `jump.blocked`, `workspace.focus`, `overlay.open.queue`). An action SHALL run with a context payload naming the focused workspace, the focused or clicked session, an optional index, the current selection, the held worktree record when one exists, and whether a key or a click triggered it. Keys and clicks that trigger the same behaviour SHALL resolve to the same action id.

#### Scenario: Key and click share an action
- **WHEN** the user presses `a` with a blocked session in another workspace, and separately clicks that session's row in the triage queue
- **THEN** both resolve to the same jump behaviour through the action registry and produce the same focus and engaged state

#### Scenario: Action list is complete
- **WHEN** the registry is listed
- **THEN** it contains an id for every garage-layer binding in spec tui-key-routing, every overlay key, every clickable tile, rail and triage target, and every restart chord

### Requirement: Keymap layer
Garage-layer keys, chord second keys and overlay keys SHALL resolve to action ids through one keymap table. The default keymap SHALL reproduce the bindings in spec tui-key-routing and spec restart exactly. Double-press arming, the `X` then `K` confirm and the one-shot `r` prefix SHALL keep their current timing, disarm rules and strip wording. Engaged-layer input SHALL NOT pass through the keymap, apart from the existing disengage combination.

#### Scenario: Defaults unchanged
- **WHEN** the wall starts with no user keymap
- **THEN** every key listed in the help overlay performs the same behaviour, with the same strip notices, as before this change

#### Scenario: Engaged input stays verbatim
- **WHEN** a tile is engaged and the user types `x`
- **THEN** the byte reaches the agent and no action runs

### Requirement: UI slots
The wall SHALL render named UI slots: `tile.title.right`, `rail.row`, `rail.header`, `strip.right`, `overlay` and `context.menu`. Each slot SHALL collect items from registered providers, where an item is styled text with an optional click action and a priority, and SHALL lay items out by priority, dropping the lowest-priority items first when space runs out. Core features that occupy these places (the tile context meter, the strip usage chip, the pit pet, the family label row) SHALL render through their slot. A click on a slot item with a click action SHALL run that action.

#### Scenario: Core renders through slots
- **WHEN** a tile shows a context meter and the strip shows the usage chip
- **THEN** both come from slot providers and the rendered frame is identical to the frame before this change

#### Scenario: Narrow strip
- **WHEN** the terminal is too narrow for every `strip.right` item
- **THEN** the lowest-priority items are dropped first and the remaining items render whole

### Requirement: Optional pit pet
The pit pet SHALL be one module behind a single interface, compiled under a default-on build feature. With the feature off the wall SHALL build and run without the pet, and its strip slot SHALL simply be empty.

#### Scenario: Build without the pet
- **WHEN** the wall is built with default features off
- **THEN** it compiles, its non-pet tests pass, and the strip renders with no pet
