# Tasks: p24-plugin-host

Order matters: group 1 changes the existing core seams and must pass the full existing test suite with no plugins present before anything plugin-specific lands. Group 2 is the contract everything else compiles against.

## 1. Core seams (TUI) - close the p22 review gaps
- [ ] 1.1 `ActionId::intern(&str)` with a process-wide interner (stays `Copy`); `plugin:action` id form; unit tests (same string gives the same pointer, core ids unchanged, `ids::ALL` stays core-only)
- [ ] 1.2 `ClickTarget { action, args }` replaces `on_click: Option<ActionId>` in `SlotItem` and `Placed`; `Laid::clicks` returns targets; `ActionCtx.args` + `with_args`; `ActionSource::Widget` and `ActionSource::Plugin`; core providers migrate with no output change (existing snapshot tests pass untouched)
- [ ] 1.3 Click regions for `tile.title.right`, `rail.row`, `rail.header`: renderers push `SlotClickRegion` into `ClickMap.slot_clicks`; `hit_targets::action_at` resolves them (overlay barrier respected) and builds `ActionCtx` from the region's session/workspace; tests incl. clicking an unfocused tile's button
- [ ] 1.4 Multiple `Placement::Filler` items in `strip.right`: sort by `x`, drop lower priority on overlap; test the pit-pet-alone case is unchanged
- [ ] 1.5 `ActionMeta` table (`dangerous`, `plugin_invocable`); dispatch arms dangerous actions from `Widget`/`Plugin` sources through `armed_action.rs` and runs them only on the confirming trigger; restart family accepts a confirmed plugin source; tests for each dangerous id, and for non-invocable ids (`app.quit`, `overlay.*`, `chord.restart`) being refused

## 2. Contract: manifest crate, protocol types, schema
- [ ] 2.1 New crate `crates/plugin` (`garage-plugin`): manifest types (header, actions, events, panes, widgets, buttons, cli, build, release), `deny_unknown_fields`, field-path error messages
- [ ] 2.2 Validation: id rules and reserved words, unique local ids, slot/widget-scope compatibility, `on_click` resolution, exactly one of `command`/`pane`, timer minimum, `[release]` requires checksums, cli name collisions, `requires` features, platforms and `api_version` checks; table-driven tests with one failing manifest per rule (incl. the "action without command" spec scenario)
- [ ] 2.3 Argv substitution (`{plugin_dir}`, `{bin}`, `{state_dir}`, `{platform}`) with tests; no shell involved
- [ ] 2.4 Widget wire types in `garage-protocol` (item, segment, style, click, scopes, limits) with round-trip JSON tests; size/sanitise validation shared by daemon
- [ ] 2.5 schemars-generated `schemas/garage-plugin.schema.json`, `claude-garage plugin schema`, and a test that fails when the committed schema is stale; schema validates the close-button example and rejects a bad one
- [ ] 2.6 Feature table and api-version support set in one place (`features()`), used by validation, load and `/api/plugin/features`

## 3. Registry and install lifecycle (works without the daemon)
- [ ] 3.1 Disk layout and `plugins.json` (atomic write, versioned, tolerant of a corrupt file with a clear error); per-plugin token file (0600)
- [ ] 3.2 GitHub source parsing (`owner/repo[/subdir]`, full URL, `--ref`) and shallow clone via `git` with timeout; symlink-escape check; commit pinning
- [ ] 3.3 Install preview renderer (identity, "not sandboxed" line, permissions with DANGEROUS marks, every command that will run, release URLs, keys/buttons/widgets/panes, update diff); golden-output tests
- [ ] 3.4 `plugin install` flow incl. confirmation, `--yes`, `--allow-dangerous`, and "nothing runs before confirm" test with a recording runner
- [ ] 3.5 Release binary download with checksum verification and per-OS asset selection; `[[build]]` runner with timeout and log capture; failure leaves no partial install; test that a listed asset skips `[[build]]`
- [ ] 3.6 `plugin link` (no build, preview still shown), `list [--json]`, `enable|disable`, `remove [--purge]`, `update [id|--all]` (staged swap, prompt only on commands/permissions/URLs change)
- [ ] 3.7 Replace the `Plugin` stub in `crates/cli/src/main.rs` with the real subcommands; clap `external_subcommand` dispatch for `[[cli]]` (core names win, interactive stdio, exit code passthrough)

## 4. Daemon runtime
- [ ] 4.1 `AppState.plugins` (`PluginHost`): load registry at start, reload endpoint `POST /api/plugins/reload`, mtime poll for linked manifests (keep last good manifest on error)
- [ ] 4.2 Process runner: argv spawn in own process group (OS seam in `os/`), context JSON on stdin and env (file fallback above 32 KiB), env vars table, log capture with run ids and rotation, timeouts with TERM then KILL, tests with a fake runner and with a real tiny script
- [ ] 4.3 `DaemonEvent` payload variants (`SessionCreated`, `SessionClosed`, `WorktreeCreated`, `WorkspaceAdded/Removed`, `PluginRunFailed`, `WidgetsChanged`, `PluginsChanged`); poller emits typed session events from its existing diff; existing SSE frames unchanged
- [ ] 4.4 Event dispatcher as an `EventBus` subscriber: per-plugin bounded queue, 4-run concurrency cap, timer events, auto-disable after repeated failures; tests for each of those and for the `session.closed` spec scenario
- [ ] 4.5 `POST /api/plugins/{id}/actions/{action}/run` (202 + run id), failure toast event, plugin API token auth middleware with soft permission checks (403 names the permission), origin-guard interplay tested
- [ ] 4.6 Widget store and routes: `PUT/DELETE /api/plugins/{id}/widgets/{widget}`, limits (413/422), token-bucket rate limit (429 + Retry-After), equal-content no-op, 200 ms coalescing, ttl expiry, `GET /api/plugins/widgets` snapshot; SSE frames `widgets` and `plugins`
- [ ] 4.7 Panes: `kind: "plugin-pane"` sessions hidden from the agent session list/poller/restore, open/close routes, singleton handling, cleanup of orphan pane sessions at start and on disable, `GET /api/plugins/panes` with attach spec
- [ ] 4.8 `POST /api/plugins/ui/action` forwarded as SSE `invoke`; `POST /api/sessions/{id}/input` with `SessionBackend::send_text` (tmux impl) gated by `sessions.input`
- [ ] 4.9 `GET /api/plugin/features` and uninstall/disable sequence (stop new triggers, kill process groups, kill panes, clear widgets, publish `PluginsChanged`)

## 5. TUI integration
- [ ] 5.1 Plugin snapshot store + SSE/refetch task (`widgets`, `plugins`, `invoke`, run-failed frames); redraw on change; refetch on reconnect
- [ ] 5.2 `PluginSlotProvider` per slot registered next to `SlotRegistry::core()`: scope selection from `SlotCtx`, style/token mapping via `ui/theme.rs`, click targets with args, ordering by (priority, install order); no-plugin case renders byte-identical to before (snapshot tests)
- [ ] 5.3 Manifest buttons as static slot items; `confirm` handling via armed state
- [ ] 5.4 Keymap rebuild on `PluginsChanged`: `default_key` via `bind`, user bindings via `bind_override`, conflict detection and reporting; `claude-garage plugin bind`
- [ ] 5.5 Dispatch: `plugin:action` to `Effect::RunPluginAction` / `Effect::OpenPluginPane`; `contexts` gating with hint toast; context JSON builder from `ActionCtx`
- [ ] 5.6 Pane rendering: `OverlayKind::PluginPane` (overlay and popup) reusing the PTY tile code, reserved close chord, tile placement in the grid (excluded from salience/triage); exit handling
- [ ] 5.7 Help overlay "Plugins" section (actions, keys, conflicts)

## 6. Docs and sample
- [ ] 6.1 Manifest reference and plugin API reference (routes, events, context JSON, limits, features), including the security section in plain words
- [ ] 6.2 `examples/plugins/close-button` (the worked example; no commands) and `examples/plugins/hello` covering an action, an event, an overlay pane and a widget in a few lines of shell/Python

## 7. Verification
- [ ] 7.1 Unit tests (manifest, validation, registry, runner, rate limits, keymap rebuild) and contract tests for every plugin route and SSE frame
- [ ] 7.2 `openspec validate p24-plugin-host --strict` passes
- [ ] 7.3 Live (main session only, with consent): install `hello` from GitHub with preview, exercise action by key, button click on an unfocused tile, event hook, widget in strip and tile title, overlay pane, `[[cli]]`; link-reload; update; remove and confirm no leftover processes or panes
- [ ] 7.4 Record results in `verification.md`
