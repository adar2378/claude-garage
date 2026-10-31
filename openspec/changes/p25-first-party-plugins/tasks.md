# Tasks: p25-first-party-plugins

## 1. Core data
- [x] 1.1 Daemon keeps closed-session records (id, workspace, title, closed at) with an API to list them (records also keep the worktree, so a restored worktree chat can still be finished)

## 2. First-party plugins (`plugins/`)
- [ ] 2.1 closed-chats (rail widget rows + `H` recent overlay)
- [ ] 2.2 jump-palette (`/` overlay pane)
- [ ] 2.3 quick-send (`s` / `S`)
- [ ] 2.4 tile-buttons (close / delete)
- [x] 2.5 usage-bar (5h / weekly, persisted)
- [x] 2.6 turn-markers
- [ ] 2.7 prune-empty-projects (`session.closed` hook)
- [ ] 2.8 cli-extras (`<folder>`, `install`, `prune`)
- [x] 2.9 pit-pet (moved out of `ui/pet.rs`, same behaviour; differences listed under 4.1 G2, G3, G5)

## 3. Core items
- [ ] 3.1 Draft and pasted-screenshot recovery
- [ ] 3.2 Mouse selection and copy
- [ ] 3.3 Sidebar hide and resize
- [ ] 3.4 Quiet startup (`GARAGE_VERBOSE=1` for full output)

## 4. API freeze
- [ ] 4.1 List every plugin API gap found in group 2 and close it in the plugin host
  - Found by usage-bar, pit-pet and turn-markers (other plugins append theirs). Status: closed in the host / worked around in the plugin / open.
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
