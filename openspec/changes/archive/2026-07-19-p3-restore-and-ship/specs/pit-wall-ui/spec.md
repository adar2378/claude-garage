## MODIFIED Requirements

### Requirement: Keybindings suppressed while typing into a terminal
Global pit-wall keybindings (`1`–`9`, `[`, `]`, `a`) SHALL NOT fire while keyboard focus is inside a terminal's input (i.e. the user is typing to send input to the running Claude Code process). Keystrokes in that state SHALL be delivered to the terminal's pty instead. The one exception is a dedicated chord, `Ctrl+\``, which SHALL blur the focused terminal and return keyboard focus to chrome-navigation mode even while the terminal has focus — it SHALL fire regardless of terminal focus, unlike every other global keybinding.

#### Scenario: Digit typed into terminal does not switch workspace
- **WHEN** a terminal has input focus and the user types `2` as part of a prompt to Claude
- **THEN** the `2` is sent to that terminal's pty as input, and the focused workspace does not change

#### Scenario: Bracket typed into terminal does not cycle focus
- **WHEN** a terminal has input focus and the user types `[` as part of program input
- **THEN** the `[` is sent to the pty and the focused terminal within the grid does not change

#### Scenario: Ctrl+` blurs the focused terminal
- **WHEN** a terminal has input focus and the user presses `Ctrl+\``
- **THEN** keyboard focus leaves the terminal and returns to chrome-navigation mode, and `Ctrl+\`` is not sent to the terminal's pty as input

## ADDED Requirements

### Requirement: Help overlay
The UI SHALL support a `?` keybinding that opens a help overlay listing all keybindings (`1`–`9`, `[`, `]`, `a`, `Ctrl+\``, `?`, `Esc`). `Esc` SHALL close the overlay when it is open. The `?` keybinding SHALL be suppressed while a terminal has keyboard focus, consistent with other global keybindings, so typing `?` into a terminal does not open the overlay.

#### Scenario: ? opens the help overlay
- **WHEN** chrome-navigation mode has focus (no terminal focused) and the user presses `?`
- **THEN** a help overlay appears listing all keybindings and their effects

#### Scenario: Esc closes the help overlay
- **WHEN** the help overlay is open and the user presses `Esc`
- **THEN** the overlay closes and the previous view (rail + grid) is visible again

#### Scenario: ? typed into a terminal does not open the overlay
- **WHEN** a terminal has input focus and the user types `?` as part of program input
- **THEN** the `?` is sent to the terminal's pty and no help overlay appears

### Requirement: Restorable sessions in the rail
The workspace rail SHALL render restorable sessions (status `restorable`) visually distinct from live sessions — dimmed, with a `⟳` glyph — and SHALL provide a restore control for each. When every session belonging to a workspace's deck is restorable, the rail SHALL additionally show a restore-all control for that workspace (e.g. the post-reboot state where tmux came back empty). Activating a restore control SHALL call the restore endpoint and, on success, cause the restored session to appear live in the grid.

#### Scenario: Restorable session renders dimmed with the ⟳ glyph
- **WHEN** `GET /api/sessions` reports `garage/kowboy/checkout` with status `restorable`
- **THEN** the rail renders that session dimmed with a `⟳` glyph instead of the usual status glyph, alongside a restore control

#### Scenario: Restore-all control appears when a workspace's entire deck is restorable
- **WHEN** all sessions belonging to workspace `kowboy` are reported `restorable` (e.g. immediately after a reboot, before anything is restored)
- **THEN** the rail shows a restore-all control for `kowboy` in addition to each session's individual restore control

#### Scenario: Restoring from the rail puts the session back in the grid live
- **WHEN** the user activates the restore control for a restorable session `garage/kowboy/checkout` while `kowboy` is the focused workspace
- **THEN** the daemon restores the session, and once restore succeeds the grid renders `garage/kowboy/checkout` as a live, interactive terminal (no longer dimmed/restorable in the rail)
