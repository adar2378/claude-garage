# Tasks: p24-plugin-host

## 1. Manifest and registry
- [ ] 1.1 Rust types for `garage-plugin.toml`, validation, generated JSON schema
- [ ] 1.2 Plugin registry on disk (installed, linked, enabled), loaded at daemon start

## 2. Install lifecycle
- [ ] 2.1 `plugin install owner/repo[/subdir] [--ref]` with summary + permission prompt
- [ ] 2.2 Per-OS release binary download; `[[build]]` runner
- [ ] 2.3 `plugin link`, `update`, `list`, `remove`

## 3. Runtime
- [ ] 3.1 Process runner for actions, events and CLI subcommands, with context JSON and logs
- [ ] 3.2 Event bus wiring for `[[events]]`
- [ ] 3.3 Panes as backend sessions (tile, overlay, popup)
- [ ] 3.4 Widget store + daemon API to push slot content; TUI slot rendering
- [ ] 3.5 Buttons bound to actions through the action registry
- [ ] 3.6 Storage dir and `/api/plugin/features`

## 4. Docs and sample
- [ ] 4.1 Manifest and plugin API reference
- [ ] 4.2 One sample plugin covering action, event, pane and widget

## 5. Verification
- [ ] 5.1 Unit tests (manifest parsing, registry, runner) and contract tests for the plugin API
- [ ] 5.2 Live: install the sample plugin from GitHub, use each surface, remove it
- [ ] 5.3 Record results in `verification.md`
