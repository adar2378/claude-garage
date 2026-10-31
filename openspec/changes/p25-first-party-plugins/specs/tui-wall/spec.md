## ADDED Requirements

### Requirement: Draft and screenshot recovery
The core SHALL keep the text a user has typed but not sent, and pasted screenshots, for each session, and SHALL offer to restore them after the wall, daemon or machine stopped unexpectedly.

#### Scenario: Draft back after a power cut
- **WHEN** the machine loses power while a typed, unsent message sits in a session's input
- **THEN** after restore the wall offers that text back for that session

### Requirement: Mouse selection and copy
The TUI SHALL let the user select text in a tile with the mouse and copy it to the system clipboard.

#### Scenario: Copy a selection
- **WHEN** the user drags over text in a tile and presses the copy key
- **THEN** the selected text is on the system clipboard and no key reaches the session

### Requirement: Sidebar hide and resize
The TUI SHALL let the user hide the rail and change its width with keys and by dragging its edge, and SHALL remember the choice across restarts.

#### Scenario: Resize persists
- **WHEN** the user widens the rail and restarts the wall
- **THEN** the rail opens at the new width
