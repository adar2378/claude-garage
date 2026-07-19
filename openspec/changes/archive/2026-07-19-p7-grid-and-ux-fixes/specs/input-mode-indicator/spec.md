# input-mode-indicator

## ADDED Requirements

### Requirement: Keys-routing chip
The app header SHALL always display a mode chip indicating where keystrokes currently go: a chrome state (e.g. `keys → garage`) when no terminal has keyboard focus, and a terminal state naming the session (e.g. `keys → checkout`) while a terminal has focus. The chip SHALL update immediately on terminal focus and blur, including blur via `Ctrl+\``.

#### Scenario: Chip flips on terminal focus and back on release
- **WHEN** the user clicks into session `checkout`'s terminal and later presses `Ctrl+\``
- **THEN** the chip reads `keys → checkout` while the terminal has focus and returns to the chrome state after the chord

### Requirement: Terminal-focus escape hint
When a terminal gains keyboard focus, the UI SHALL show a transient hint telling the user that keys now go to that session and that `Ctrl+\`` returns them to chrome navigation. The hint SHALL dismiss on its own and SHALL NOT require interaction.

#### Scenario: Hint appears on first focus
- **WHEN** the user clicks inside a terminal
- **THEN** a transient hint appears naming the session and the `Ctrl+\`` escape chord, then disappears without user action

### Requirement: Footer key strip
The UI SHALL display a persistent, single-line footer strip listing the core chrome keybindings (at minimum `?`, `a`, `1`–`9`) and the current key-routing state, so the keybindings are discoverable without first knowing that `?` exists. The strip SHALL NOT be a keyboard-focus target.

#### Scenario: Core keys are visible without opening help
- **WHEN** the pit wall is rendered in its normal state
- **THEN** the footer strip shows the core keybinding hints and reflects whether keys currently go to chrome or to a named terminal
