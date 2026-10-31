# Tasks: p24-plugin-host

Order matters: group 1 changes the existing core seams and must pass the full existing test suite with no plugins present before anything plugin-specific lands. Then the contract and daemon host core, then bundled first-party plugins (the default path), then widgets/panes/actions, CLI, and last GitHub install/update. Verification closes.

## 1. TUI seam gaps - close the p22 review gaps
- [x] 1.1 `ActionId::intern(&str)` with a process-wide interner (stays `Copy`); `plugin:action` id form; unit tests (same string gives the same pointer, core ids unchanged, `ids::ALL` stays core-only)
- [x] 1.2 `ClickTarget { action, args }` replaces `on_click: Option<ActionId>` in `SlotItem` and `Placed`; `Laid::clicks` returns targets; `ActionCtx.args` + `with_args`; `ActionSource::Widget` and `ActionSource::Plugin`; core providers migrate with no output change (existing snapshot tests pass untouched)
- [x] 1.3 Click regions for `tile.title.right`, `rail.row`, `rail.header`: renderers push `SlotClickRegion` into `ClickMap.slot_clicks`; `hit_targets::action_at` resolves them (overlay barrier respected) and builds `ActionCtx` from the region's session/workspace; a `strip.right` click on a session-scoped core action targets the focused session; tests incl. clicking an unfocused tile's button
- [x] 1.4 Multiple `Placement::Filler` items in `strip.right`: sort by `x`, drop lower priority on overlap; test the pit-pet-alone case is unchanged
- [x] 1.5 `ActionMeta` table (`dangerous`, `plugin_invocable`); confirm rule: sources `Plugin`/`Widget` only ARM a dangerous action through `armed_action.rs`, only `Key`/`Mouse`/`Chord` CONFIRM it; restart family accepts a confirmed plugin source only from that path; tests for each dangerous id, for non-invocable ids (`app.quit`, `overlay.*`, `chord.restart`) being refused, and for "plugin triggers twice never executes"
- [x] 1.6 New core action `session.delete` (forget: `DELETE /api/sessions/{*id}?meta=1` semantics), `dangerous` and `plugin_invocable`, with TUI confirm; core actions take their target only from the click region / `target`, never from `args` (test: crafted `args.session` is ignored)
- [ ] 1.7 New slot `tile.gutter`: one-column gutter allocated only when a session has gutter items, row-addressed items mapped through the scrollback offset, click regions; tiles with no gutter items lay out byte-identically (snapshot tests)
- [x] 1.8 New slot `rail.extra_rows`: extra rows per workspace with style and click region; `rail_target_at` row maths counts them; no-extra-rows case unchanged (snapshot tests)
- [ ] 1.9 Animated `SlotItem` (frame list + interval clamped 100..=2000 ms): frame chosen from the TUI tick, redraw tick only while a visible animated item exists; test no extra wake-ups without animated items
- [ ] 1.10 Slot geometry (`strip.right` width and filler range, max item columns for other slots) computed by the renderer and exposed for the plugin snapshot; tests on narrow and wide terminals

## 2. Contract: manifest crate, protocol types, schema
- [ ] 2.1 New crate `crates/plugin` (`garage-plugin`): manifest types (header, actions, events, panes, widgets, buttons, cli incl. `fallback`, build, release), `deny_unknown_fields`, field-path error messages
- [ ] 2.2 Validation: id rules and reserved words, unique local ids, slot/widget-scope compatibility (incl. `tile.gutter`, `rail.extra_rows`), `on_click` resolution, exactly one of `command`/`pane`, timer minimum, `[release]` requires checksums, cli name collisions (`install` is not reserved), at most one `fallback = true` across plugins (first-party wins), `requires` features, platforms and `api_version` checks; table-driven tests with one failing manifest per rule (incl. the "action without command" spec scenario)
- [ ] 2.3 Argv substitution (`{plugin_dir}`, `{bin}`, `{state_dir}`, `{platform}`) with tests; no shell involved
- [ ] 2.4 Widget wire types in `garage-protocol` (item, segment, style, click, scopes, animated `{frames, frame_ms}`, gutter row items, extra-row items, limits) with round-trip JSON tests; size/sanitise validation shared by daemon
- [ ] 2.5 schemars-generated `schemas/garage-plugin.schema.json`, `claude-garage plugin schema`, and a test that fails when the committed schema is stale; schema validates the close-button example and rejects a bad one
- [ ] 2.6 Feature table and api-version support set in one place (`features()`), used by validation, load and `/api/plugin/features` (incl. `widgets.animated`, `widgets.slot_geometry`, `cli.fallback`, `session.turn`)

## 3. Daemon plugin host core
- [ ] 3.1 Disk layout and `plugins.json` (atomic write, versioned, tolerant of a corrupt file with a clear error); per-plugin token file (0600)
- [ ] 3.2 `AppState.plugins` (`PluginHost`): load registry at start, `POST /api/plugins/reload`, mtime poll for linked manifests (keep last good manifest on error; a change that adds permissions keeps the last good manifest and toasts "plugin X wants new permissions - run `claude-garage plugin link` to approve")
- [ ] 3.3 Process runner: argv spawn in own process group (OS seam in `os/`), context JSON on stdin and env (file fallback above 32 KiB) including `ui.slots` geometry, env vars table, log capture with run ids and rotation, timeouts with TERM then KILL, tests with a fake runner and with a real tiny script
- [ ] 3.4 `DaemonEvent` payload variants (`SessionCreated`, `SessionClosed{reason: user|crash|daemon-restart}`, `SessionTurn` from the Stop hook with session id, timestamp and transcript offset if available, `WorktreeCreated`, `WorkspaceAdded/Removed`, `PluginRunFailed`, `WidgetsChanged{plugin, rev}`, `PluginsChanged`); closing-intent marker so reasons are correct; existing SSE frames unchanged
- [ ] 3.5 Closed-session records in core (bounded per workspace, restore/forget removes), `GET /api/workspaces/{name}/closed`, included in the plugin snapshot
- [ ] 3.6 Event dispatcher as an `EventBus` subscriber: per-plugin bounded queue, 4-run concurrency cap, timer events, auto-disable after repeated failures; tests for each of those, for the `session.closed` (with reason) and `session.turn` spec scenarios
- [ ] 3.7 `POST /api/plugins/{id}/actions/{action}/run` (202 + run id), failure toast event, plugin API token auth middleware with soft permission checks (403 names the permission), origin-guard interplay tested
- [ ] 3.8 `POST /api/sessions/input` body `{id, text}`, token REQUIRED (401 without), permission `sessions.input`, `SessionBackend::send_text` (tmux impl); tests incl. missing token and wrong permission
- [ ] 3.9 `GET /api/plugin/features` and uninstall/disable sequence (stop new triggers, kill process groups, kill panes, clear widgets, publish `PluginsChanged`)

## 4. Bundled first-party plugins (default path)
- [ ] 4.1 Workspace layout `plugins/<name>/` crates and a shared tiny helper crate for plugin API calls; release build compiles them; package scripts ship binaries and manifests under `plugins/`; no Node at runtime
- [ ] 4.2 `bundled` source kind in `plugins.json`; first-run seeding with defaults (on: `usage-bar`, `closed-chats`, `quick-send`, `tile-buttons`, `pit-pet`; off: `jump-palette`, `turn-markers`, `cli-extras`, `prune-empty-projects`), `seeded` marker, user-disabled never re-enabled, bundled not removable (disable only); tests with a temp `GARAGE_DIR`
- [ ] 4.3 First-party plugins implemented against the contract: `usage-bar`, `tile-buttons`, `quick-send`, `closed-chats` (`rail.extra_rows`, restore, `session.delete` with confirm), `pit-pet` (animated frames, slot geometry, pushes on state change only), `turn-markers` (`session.turn`, `tile.gutter`), `jump-palette`, `cli-extras` (`fallback = true`), `prune-empty-projects`
- [ ] 4.4 `plugin list` shows bundled plugins with source `bundled` and their permissions

## 5. Widgets, panes, actions (daemon and TUI)
- [ ] 5.1 Widget store and routes: `PUT/DELETE /api/plugins/{id}/widgets/{widget}`, limits (413/422), animated and gutter/extra-row forms, token-bucket rate limit (429 + Retry-After), equal-content no-op, 200 ms coalescing, ttl expiry, per-plugin `rev`, `GET /api/plugins/widgets[?plugin=]`; `click.action` core ids checked against `core.actions` (403); SSE frames `widgets` (`{plugin, rev}`) and `plugins`
- [ ] 5.2 Plugin snapshot store + SSE/refetch task in the TUI: fetch only the named plugin's widgets when its `rev` changed, full snapshot on start/`plugins`/reconnect; redraw on change
- [ ] 5.3 `PluginSlotProvider` per slot (incl. `tile.gutter`, `rail.extra_rows`) registered next to `SlotRegistry::core()`: scope selection from `SlotCtx`, style/token mapping via `ui/theme.rs`, click targets with args, ordering by (priority, install order); `SlotItem`s built once per snapshot and cached, not per frame; no-plugin case renders byte-identical (snapshot tests)
- [ ] 5.4 Manifest buttons as static slot items; `confirm` handling via armed state
- [ ] 5.5 Keymap rebuild on `PluginsChanged`: `default_key` via `bind`, user bindings via `bind_override`, conflict detection and reporting; `claude-garage plugin bind`
- [ ] 5.6 Dispatch: `plugin:action` to `Effect::RunPluginAction` / `Effect::OpenPluginPane`; `contexts` gating with hint toast; context JSON builder from `ActionCtx`
- [ ] 5.7 Panes: `kind: "plugin-pane"` sessions hidden from the agent session list/poller/restore, open/close routes, singleton handling, orphan cleanup, `GET /api/plugins/panes` with attach spec; TUI `OverlayKind::PluginPane` (overlay and popup) reusing the PTY tile code, reserved close chord, tile placement (excluded from salience/triage), exit handling
- [ ] 5.8 `POST /api/plugins/ui/action` with `{action, args, target: {session?, workspace?}}` forwarded as SSE `invoke`; core actions take target only from `target`; a plugin invoking a dangerous action twice only arms it (test)
- [ ] 5.9 Help overlay "Plugins" section (actions, keys, conflicts)

## 6. CLI
- [ ] 6.1 Replace the `Plugin` stub in `crates/cli/src/main.rs` with the real subcommands (`list`, `enable|disable`, `bind`, `logs`, `schema`); clap `external_subcommand` dispatch for `[[cli]]` (core names win, interactive stdio, exit code passthrough)
- [ ] 6.2 `[cli] fallback = true`: unknown first word goes to the single fallback plugin with the full argv; refused when two plugins claim it (first-party wins); no fallback keeps the clap error; `install` is not a reserved core name; tests

## 7. GitHub install, release and update (ships in 1.0; ecosystem is p27)
- [ ] 7.1 GitHub source parsing (`owner/repo[/subdir]`, full URL, `--ref`) and shallow clone via `git` with timeout; symlink-escape check; commit pinning
- [ ] 7.2 Install preview renderer (identity, "not sandboxed" line, permissions with DANGEROUS marks, every command that will run, release URLs, keys/buttons/widgets/panes, update diff, and the plain line that code-only updates apply without a prompt); golden-output tests
- [ ] 7.3 `plugin install` flow incl. confirmation, `--yes`, `--allow-dangerous`, and "nothing runs before confirm" test with a recording runner
- [ ] 7.4 Release binary download with checksum verification and per-OS asset selection; `[[build]]` runner with timeout and log capture; failure leaves no partial install; test that a listed asset skips `[[build]]`
- [ ] 7.5 `plugin link` (no build, preview still shown; also the approval path for added permissions), `remove [--purge]`, `update [id|--all]` (staged swap, prompt only on commands/permissions/URLs change)

## 8. Docs and samples
- [ ] 8.1 Manifest reference and plugin API reference (routes, events incl. `session.turn` and close reasons, context JSON, slot geometry, animated widgets, limits, features), including the security section in plain words
- [ ] 8.2 `examples/plugins/close-button` (the worked example; no commands) and `examples/plugins/hello` covering an action, an event, an overlay pane and a widget in a few lines of shell/Python

## 9. Verification
- [ ] 9.1 Unit tests (manifest, validation, registry, runner, rate limits, keymap rebuild, confirm rule) and contract tests for every plugin route and SSE frame
- [ ] 9.2 `openspec validate p24-plugin-host --strict` passes
- [ ] 9.3 Live (main session only, with consent): fresh `GARAGE_DIR` seeds the bundled defaults; pit pet animates with no plugin ticking; turn markers; closed chats row restore and delete; quick send; then install `hello` from GitHub with preview, key action, button click on an unfocused tile, event hook, widget, overlay pane, `[[cli]]` and fallback; link-reload with a permission-adding manifest; update; remove and confirm no leftover processes or panes
- [ ] 9.4 Record results in `verification.md`
