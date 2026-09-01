# tui-key-routing (delta)

## ADDED Requirements

### Requirement: Enhanced-keyboard passthrough
On outer terminals that support the kitty keyboard protocol, the wall SHALL
push the disambiguate enhancement flag on entry and pop it on every exit
path (quit, error, panic). The engaged encoder SHALL emit the kitty CSI-u
form for modifier combinations the legacy encoding cannot express — at
minimum Shift+Enter (`ESC[13;2u`) and Ctrl+Enter (`ESC[13;5u`) — while
Option+Enter keeps the legacy `ESC CR` and plain Enter stays `\r`. The
daemon SHALL apply Claude Code's documented tmux extended-keys server
config (`extended-keys on`, `terminal-features 'xterm*:extkeys'`)
idempotently at boot and on session creation, so the sequence reaches the
pane's application.

#### Scenario: Shift+Enter reaches Claude Code as a newline
- **WHEN** the user presses Shift+Enter in an engaged tile on a kitty-capable outer terminal
- **THEN** the pane receives `ESC[13;2u` via tmux extended keys, and Claude Code inserts a newline instead of submitting

#### Scenario: Legacy terminals degrade gracefully
- **WHEN** the outer terminal does not support keyboard enhancement
- **THEN** no flags are pushed and every previously working key encodes exactly as before

### Requirement: Click opens links
A left click that lands on an `http://`/`https://` URL in a tile's rendered
terminal text (live or frozen) SHALL open that URL with the OS opener and
show a notice, and SHALL NOT focus or engage the tile. Trailing prose
punctuation is not part of the URL. Clicks not on a URL SHALL route exactly
as before (focus/engage/restore).

#### Scenario: Clicking a URL in a response opens the browser
- **WHEN** Claude Code prints `https://example.com/docs` in a tile and the user clicks inside that text
- **THEN** the OS opens the URL and the tile's engagement state is unchanged

#### Scenario: Ordinary clicks still engage
- **WHEN** the user clicks tile text that is not a URL
- **THEN** the tile focuses and engages exactly as before this change
