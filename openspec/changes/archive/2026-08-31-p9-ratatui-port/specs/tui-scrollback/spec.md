# tui-scrollback (delta)

## MODIFIED Requirements

### Requirement: History depth
Because tmux attach clients render on the alternate screen, a tile's embedded emulator never accumulates scrollback; frozen history SHALL therefore be seeded from tmux itself. On entering the frozen view, the TUI SHALL take a `tmux capture-pane -e` snapshot anchored at an absolute history position (`#{history_size}`-based, immune to live growth by construction) covering at least 1,000 lines of pane history, and page within that snapshot. Returning to live SHALL resume the live attach view. tmux copy-mode SHALL never be entered on the underlying session.

#### Scenario: Frozen view shows pane history
- **WHEN** a pane has 500 lines of scrolled-off history and the user enters the frozen view
- **THEN** paging up shows those historical lines from the capture snapshot, and the visible content does not move while live output continues

#### Scenario: Copy-mode never triggered
- **WHEN** the user scrolls a tile while another client is attached to the same session
- **THEN** `#{pane_in_mode}` remains 0 for that pane throughout
