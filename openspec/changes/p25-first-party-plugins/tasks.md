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
  - G2 No theme token for the needs-input colour: amber is deliberately unreachable (`warn` is not amber), so the pet's lit `!` changed colour. **Open:** a maintainer call; one named token (for example `needs-input`) would restore it.
  - G3 No state for a dead daemon: the pet's "box" mood cannot be drawn because a plugin cannot run, and its widget stays frozen on the last frame. **Open:** the TUI could drop or replace plugin widgets while the SSE stream is down.
  - G4 `core.actions` is all-or-nothing and flagged DANGEROUS: the pet needs only `jump.blocked`, yet the first default-on bundled plugin that asks for permissions carries the dangerous one. **Open:** per-action grants.
  - G5 No plugin notice (toast) API: the `P` arrival text became the pet's caption and "the strip is quiet again" is lost. **Open:** `POST /api/plugins/ui/notice`.
  - G6 No `usage.changed` event: usage-bar polls on a 60 s timer plus `session.turn`. **Open (small):** publish it from the statusline route.
  - G7 Animated sets are loops on the wall clock, not sequences: a plugin that wants a continuous motion has to know the TUI clock (epoch ms) and lay frames out by `floor(t / frame_ms) mod len`. **Worked around** (pit-pet's rolling window, renewed by a 5 s timer); a `play once from arrival` mode would remove the need.
  - G8 A plugin cannot pause its own timer: pit-pet runs a 5 s timer even while no pet is selected (the run exits at once). **Open.**
  - G9 No delayed one-shot: taking a caption down after 3 or 4 s needs a detached child process. **Worked around**; a `[[events]] after_secs` or a `ui/schedule` call would be cleaner.
  - G10 No per-plugin settings or user env: the old `GARAGE_PET_ASCII` and `GARAGE_PET_CHATTER_MS` cannot reach a plugin (the environment is cleared). The pup's ASCII faces are now a marker file `ascii` in the plugin's state dir; the chatter override is dropped. **Open.**
  - G11 The plugin does not know the garage directory (no `GARAGE_DIR` in its environment): pit-pet finds the old `wall.json` by walking up from its state dir. **Worked around**; export `GARAGE_DIR`.
  - G12 Up to 4 runs of one plugin run at once and share its state: **Worked around** with `StateLock` in the SDK; a per-entry `serial = true` would be cleaner.
  - G13 The daemon cannot say where a turn ended in the scrollback (the wall owns the scrollback). **Closed in the wall:** `tile.gutter` items address a turn by the `at` of its `session.turn` event (`turn`), and `ui::turn_rows` places it on first draw of a live tile (not if older than 10 s).
- [ ] 4.2 Freeze plugin API v1 and document it

## 5. Verification
- [ ] 5.1 Unit and contract tests per plugin
- [ ] 5.2 Live: each plugin on macOS and Linux; pit pet behaves as before
- [ ] 5.3 Record results in `verification.md`
