# Tasks: p14-input-fidelity

- [x] 1. `term.rs`: push kitty disambiguate flag (guarded by `supports_keyboard_enhancement`), pop on restore
- [x] 2. `encode.rs`: CSI-u for Shift/Ctrl+Enter; Option+Enter and plain Enter unchanged (+tests)
- [x] 3. `daemon/src/tmux.js`: `ensureExtendedKeys()` — `extended-keys on` + append `xterm*:extkeys` once; called from daemon boot and `createSession`
- [x] 4. `input/links.rs`: `url_at` (+tests); `registry.row_text`; `runtime.rs` click router opens URLs before focus/engage
- [x] 5. `cargo test` (393) and `npm test` (113) green
- [x] 6. E2E: live tmux server verified (`extended-keys on`, `xterm*:extkeys` appended exactly once across two runs); rebuilt TUI boots against real sessions and quits cleanly (EXIT=0). Remaining human check: Shift+Enter newline + URL click in iTerm after restarting the TUI on ≥0.3.3.
