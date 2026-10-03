## REMOVED Requirements

### Requirement: Interactive terminal in the browser
**Reason**: The browser web wall is removed; claude-garage is TUI-only (p17-tui-only).
**Migration**: Use `npx claude-garage` (the Rust TUI). See the tui-* specs for the terminal equivalents.

### Requirement: WebSocket upgrades validate Origin
**Reason**: The browser web wall is removed; claude-garage is TUI-only (p17-tui-only).
**Migration**: No WebSocket endpoint remains; the TUI attaches to tmux directly through its own PTY.
