# Proposal: p14-input-fidelity

## Why

Claude Code's own keybindings die inside the TUI: Shift+Enter (newline)
submits instead, and clicking a URL in a response does nothing. The p8
"byte-exact passthrough" promise only covered what the legacy keyboard
encoding can express — Shift+Enter isn't in it (kitty protocol, `ESC[13;2u`),
and our mouse capture starves the outer terminal of the clicks its linkifier
needs.

## What Changes

- **Enhanced-keyboard passthrough**: the wall pushes the kitty disambiguate
  flag on terminals that support it (popped on every exit path), the encoder
  emits CSI-u for modified Enter (Shift `13;2u`, Ctrl `13;5u`; Option+Enter
  stays legacy `ESC CR`), and the daemon idempotently applies Claude Code's
  documented tmux config (`extended-keys on` + `terminal-features
  'xterm*:extkeys'`, server options) at boot and on every session spawn — so
  the sequence survives outer terminal → wall → tmux → Claude Code. This
  also fixes Shift+Enter for plain `tmux attach` users of garage sessions.
- **Click opens links**: a left click landing on an `http(s)://` URL in a
  tile's grid text (live or frozen) opens it via macOS `open` and shows a
  notice, instead of focusing/engaging the tile. Non-URL clicks route
  exactly as before.

## Capabilities

### Modified Capabilities
- `tui-key-routing`: adds "Enhanced-keyboard passthrough" and "Click opens
  links".
- `session-lifecycle` (daemon): tmux server gains the extended-keys config,
  applied idempotently.

## Impact

- Wall: `term.rs`, `input/encode.rs`, new `input/links.rs`,
  `ui/registry.rs` (row_text), `runtime.rs` click router.
- Daemon: `tmux.js` (`ensureExtendedKeys`), called from `index.js` boot and
  `createSession`.
- No breaking changes; terminals without kitty support behave as before.
