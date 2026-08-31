# tui-scrollback Specification

## Purpose
TBD - created by archiving change p8-nocterm-tui. Update Purpose after archive.
## Requirements
### Requirement: Frozen local history view
While engaged, Shift+PageUp SHALL enter a local, read-only history view of that tile anchored at an absolute buffer position: new live output SHALL NOT move the visible content while scrolled. Shift+PageDown SHALL page toward live; reaching the tail SHALL return the tile to live-follow mode. The history view SHALL be local to the TUI — tmux copy-mode SHALL never be entered on the underlying session.

#### Scenario: View frozen under live output
- **WHEN** the user scrolls up two pages in a tile whose session is streaming output
- **THEN** the visible lines do not change while new output continues to arrive, and a marker indicates the tile is not live

#### Scenario: Copy-mode never triggered
- **WHEN** the user scrolls a tile while another client is attached to the same tmux session in iTerm
- **THEN** the iTerm view never enters copy-mode and never freezes

### Requirement: Return to live
A visible affordance (e.g. `↓ back to live`, with a count of lines arrived since freezing) SHALL be shown while scrolled. Pressing End or scrolling past the tail SHALL snap the tile back to live-follow, and any key that is forwarded to the PTY (typing) SHALL also snap back to live first.

#### Scenario: Typing snaps to live
- **WHEN** the user is scrolled up in an engaged tile and types a character
- **THEN** the tile returns to live-follow and the character is forwarded to the PTY

### Requirement: Mouse wheel scrolling
Wheel-up over an unengaged tile SHALL open the same frozen history view for that tile without engaging it; wheel over an engaged tile SHALL be forwarded to the application (which handles its own scrolling in the alternate screen). Wheel-down at the tail SHALL return to live.

#### Scenario: Peek without engaging
- **WHEN** the user wheel-scrolls over an unengaged tile
- **THEN** that tile shows frozen history, the key layer is unchanged, and no bytes are sent to the PTY

### Requirement: History depth
Because tmux attach clients render on the alternate screen, a tile's embedded emulator never accumulates scrollback; frozen history SHALL therefore be seeded from tmux itself. On entering the frozen view, the TUI SHALL take a `tmux capture-pane -e` snapshot anchored at an absolute history position (`#{history_size}`-based, immune to live growth by construction) covering at least 1,000 lines of pane history, and page within that snapshot. Returning to live SHALL resume the live attach view. tmux copy-mode SHALL never be entered on the underlying session.

#### Scenario: Frozen view shows pane history
- **WHEN** a pane has 500 lines of scrolled-off history and the user enters the frozen view
- **THEN** paging up shows those historical lines from the capture snapshot, and the visible content does not move while live output continues

#### Scenario: Copy-mode never triggered
- **WHEN** the user scrolls a tile while another client is attached to the same session
- **THEN** `#{pane_in_mode}` remains 0 for that pane throughout

