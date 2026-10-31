# Tasks: p25-first-party-plugins

## 1. Core data
- [x] 1.1 Daemon keeps closed-session records (id, workspace, title, closed at) with an API to list them (records also keep the worktree, so a restored worktree chat can still be finished)

## 2. First-party plugins (`plugins/`)
- [x] 2.1 closed-chats (rail widget rows + `H` recent overlay)
- [x] 2.2 jump-palette (`/` overlay pane)
- [x] 2.3 quick-send (`s` / `S`)
- [x] 2.4 tile-buttons (close and delete buttons)
- [x] 2.5 usage-bar (5h / weekly, persisted)
- [x] 2.6 turn-markers
- [x] 2.7 prune-empty-projects (`session.closed` hook)
- [x] 2.8 cli-extras (`<folder>`, `install`, `prune`)
- [x] 2.9 pit-pet (moved out of `ui/pet.rs`, same behaviour; differences listed under 4.1 G2, G3, G5)

## 3. Core items
- [x] 3.1 Draft and pasted-screenshot recovery
- [x] 3.2 Mouse selection and copy
- [x] 3.3 Sidebar hide and resize
- [x] 3.4 Quiet startup (`GARAGE_VERBOSE=1` for full output)

## 4. API freeze
- [ ] 4.1 List every plugin API gap found in group 2 and close it in the plugin host
  Gaps found by closed-chats, jump-palette, quick-send, tile-buttons, prune-empty-projects and cli-extras (usage-bar, pit-pet and turn-markers list theirs after). Closed in the host:
  - [x] `DELETE /api/sessions/{id}?meta=1` published no plugin-visible event, so a forgotten closed chat stayed in the rail: new additive event `session.forgotten {id, workspace}` (manifest enum, features list, schema, SSE frame).
  - [x] The same route answered 404 for a deliberately closed session (a close removes the resume metadata, only the closed record is left), so `session.delete` on a closed-chat row could never work: it now forgets a closed-only record.
  - [x] The `plugins/*` workspace glob refuses a plugin directory with no `Cargo.toml`, and `stage-plugins.sh` / `release.yml` demanded `bin/<id>` for every plugin: manifest-only plugins (tile-buttons) are `exclude`d from the workspace and packaged as manifest only.
  - [x] A `[[cli]]` command that calls the daemon API failed when no daemon was running (`claude-garage ~/dev/app` on a cold start): new `[[cli]] needs_daemon = true`; `claude-garage` runs the launcher's start-or-replace daemon check first (`launcher::ensure_daemon`).
  - [x] A CLI plugin could not hand over to the wall (it does not know where `claude-garage` is). First closed with `GARAGE_BIN`, which let the plugin run the wall with the plugin's token and run context in its environment. Replaced by a hand-over: the plugin writes `{"open_wall": true}` to the file named by `GARAGE_HANDOVER` (SDK `request_wall()`) and exits 0, and the launcher runs `tui_main` in its own environment. `GARAGE_BIN` is gone. A failed run (non-zero exit) never opens the wall.
  - [x] The SDK had no way to call core routes, ask the wall to focus something, type into a session, read the run context in a typed way, fuzzy-match, draw a picker or prompt in a pane, or fake the daemon in tests: `Api::get/post/put/delete/ui_action/session_input`, `Context`, `fuzzy`, `tui` (feature) and `testing` (feature).
  Found in review of the action plugins, all closed:
  - [x] quick-send could type into a session that was waiting on a permission prompt or question and answer it by accident. `POST /api/sessions/input` takes `if_status: {"not": ["needs-input"]}`, checked against the live status right before the keys go out (409 when it matches, nothing typed); `S` skips needs-input sessions and the popup names them ("skipped: app/fix - waiting for input"); `s` on one shows a warning that needs an extra `y`.
  - [x] prune could remove a project that gained a session between the check and the delete, and `claude-garage prune` removed without listing or asking. `DELETE /api/workspaces/{name}?if_empty=1` decides and removes under one registry lock and answers 409 `{error, reason}` for a live or restorable session or closed records. `drop_closed=1` (used by prune-empty-projects, acting on a chat the user just closed, whose record is by then written) lets the records go with the project; `min_age_secs=N` (used by `claude-garage prune`, 600) spares a project registered less than N seconds ago, so workspaces now remember `registeredAt`. `claude-garage prune` lists what it would remove, asks `[y/N]` (or needs `--yes`; with no terminal and no `--yes` it removes nothing), and leaves projects that still have closed chats.
  - [x] The `session.closed` event could beat the closed record to the registry, so closed-chats polled for it. The poller now publishes an internal `SessionClosing`; the recorder writes the record and only then publishes `session.closed` (also for records rebuilt after the bus lagged). The retry loop in closed-chats is gone, and its syncs run one at a time (lock file) with a newer sync superseding an older one, so stale rows can no longer overwrite fresh ones.
  - [x] A tile delete button could not work (above). `DELETE /api/sessions/{id}?forget=1` kills a live session and keeps no closed record or resume data, publishing `session.forgotten`; the wall's `session.delete` on a live tile calls it behind the confirm gate ("Delete ws/label? (kills it, no history)"); tile-buttons has a `del` button next to `x`. A worktree session's worktree record is returned as for a close, so it can still be finished.
  - [x] `claude-garage <typo>` started or replaced the daemon because the fallback plugin needs it. The launcher now checks the word is an existing folder (`~`, `~user`, relative, absolute) before `ensure_daemon`; a typo gets the usual error and starts nothing. Registered folders are compared by canonical path on both sides, and a path that is not UTF-8 is refused with a message that shows it.
  Also closed: a CLI plugin's own `claude-garage` child (cli-extras auto-open) found the wrong daemon under a custom `GARAGE_PORT` / `GARAGE_DIR` because CLI runs get a scrubbed environment: those settings (`GARAGE_PORT`, `GARAGE_DIR`, `GARAGE_TMUX_SOCKET`, `GARAGE_TMUX_CONTROL`, `GARAGE_BACKEND`) are now passed through.
  Noted, not changed:
  - A manifest `confirm = true` on an action applies to plugin-origin triggers (clicks, `ui/action`) only; a key press runs the action. quick-send therefore confirms inside its popup (Enter twice) instead of using it.
  - A plugin cannot read focus changes, so jump-palette cannot highlight the current session; it lists them all.
  - Widgets are in memory: closed-chats re-syncs on `daemon.started`. Persisting widgets across a daemon restart stays a v1 non-goal.
  - Rail limit: 8 extra rows per workspace, so a workspace shows its 8 newest closed chats; `H` lists up to the 20 kept per workspace.
  Found by usage-bar, pit-pet and turn-markers (other plugins append theirs). Status: closed in the host / worked around in the plugin / open.
  - G1 Per-frame filler column: an animated item had one `placement.x`, so a walking pet could not move. **Closed:** optional `frame_x` on animated filler items (feature `widgets.frame_x`, validated in `garage-protocol`, applied by the slot registry).
  - G2 No theme token for the needs-input colour: amber is deliberately unreachable (`warn` is not amber). **Closed:** the token `needs-input` (resolved at build time as a marker; the slot provider renders it amber only while a session needs input, plain text otherwise). The pet's lit `!` is amber again.
  - G3 No state for a dead daemon: the pet's "box" mood cannot be drawn because a plugin cannot run. **Worked around:** every pet push carries `ttl_secs = 15`, so a pet whose plugin or daemon is gone disappears instead of freezing; the "box" mood itself stays open (the TUI could draw its own while the SSE stream is down).
  - G4 `core.actions` is all-or-nothing and flagged DANGEROUS. **Closed:** permission `ui.navigate` (not dangerous) allows the navigation ids `jump.blocked`, `session.focus`, `workspace.focus`, `view.cycle`, `view.focus` from widget clicks, `ui/action` and manifest buttons; `core.actions` stays for the rest. pit-pet now asks for `ui.navigate` only and puts `click.action: "jump.blocked"` on its alert frames (the wall resolves a click per frame, so the jump needs no run of the plugin).
  - G5 No plugin notice (toast) API. **Closed:** `POST /api/plugins/{id}/notice {text, ttl_ms?}` with permission `ui.notice` (not dangerous): text cleaned and cut to 120 characters, `ttl_ms` 1 to 10 s (5 s default), one a second with a burst of three, published as the existing `plugin_notice` frame. pit-pet uses it for "the strip is quiet again".
  - G6 No `usage.changed` event: usage-bar polls on a 60 s timer plus `session.turn`. **Open (small):** publish it from the statusline route.
  - G7 Animated sets are loops on the wall clock, not sequences. **Worked around** (pit-pet's rolling window of 24 frames at 300 ms = 7.2 s, pushed about every 6 s by the timer, which has a 5 s minimum); a `play once from arrival` mode would remove the need.
  - G8 A plugin cannot pause its own timer: pit-pet runs a 6 s timer even while no pet is selected. **Worked around:** a run with no pet and nothing on the strip exits at once, before taking the lock or writing any file. A real pause stays open.
  - G9 No delayed one-shot: taking a caption down after 3 or 4 s needs a detached child process. **Worked around**; a `[[events]] after_secs` or a `ui/schedule` call would be cleaner.
  - G10 No per-plugin settings or user env: the old `GARAGE_PET_ASCII` and `GARAGE_PET_CHATTER_MS` cannot reach a plugin (the environment is cleared). The pup's ASCII faces are now a marker file `ascii` in the plugin's state dir; the chatter override is dropped. **Open.**
  - G11 The plugin does not know the garage directory (no `GARAGE_DIR` in its environment): pit-pet finds the old `wall.json` by walking up from its state dir. **Worked around**; export `GARAGE_DIR`. The wall keeps the legacy `pet` key (and any other key it does not own) when it saves views, so the migration source survives.
  - G12 Up to 4 runs of one plugin run at once and share its state: **Worked around** with `StateLock` in the SDK; a lock wait that times out is the distinct `Error::LockHeld`, which pit-pet turns into a skipped run (exit 0) so it never counts toward the five-failure auto-disable. A per-entry `serial = true` would be cleaner.
  - G13 The daemon could not say where a turn ended in the scrollback. **Closed in the daemon:** when it handles the Stop hook it asks the backend for the session's `history_size` and `cursor_y` (tmux `display-message`; host backend: a new control frame answered from its vt100 screen) and puts the absolute `row` (and `history_limit`) in the `session.turn` event. turn-markers pushes `{row}` items for the last 32 turns per session, capped by size (under 12 KiB), and the wall draws them and drops the ones below `history_size - history_limit`. The first-seen placement in the wall is gone. **Known limit:** once a pane's history is full, tmux trims its oldest lines and every absolute row shifts; a marker is then drawn on a later line than it belongs to, or not at all. tmux has no counter of trimmed lines to correct this.
- [ ] 4.2 Freeze plugin API v1 and document it

## 5. Verification
- [ ] 5.1 Unit and contract tests per plugin
- [ ] 5.2 Live: each plugin on macOS and Linux; pit pet behaves as before
- [ ] 5.3 Record results in `verification.md`
