# Tasks: p10-views-and-subtitles

## 1. Daemon: pane title

- [x] 1.1 Extend the existing list-panes format with `#{pane_title}`; normalization (empty/hostname/shell → null) with unit tests
- [x] 1.2 `title` on GET /api/sessions entries (null for restorable); tests

## 2. Wall state: views

- [x] 2.1 View model + store transitions (detach_focused, cycle_view, focus_view_of) with default-view-as-complement semantics; per-view grid cap/LRU; unit tests incl. views.js semantic parity cases
- [x] 2.2 Persistence: wall.json load/save/prune (GARAGE_DIR-aware, disposable); tests
- [x] 2.3 Cross-view salience: jump/queue/rail-click focus the target's view; tests

## 3. Wall UI

- [x] 3.0 State: `move_focused_to_view(name)` store transition + `D` command mapping (per the added Move-to-a-group requirement); tests
- [x] 3.1 View strip (≥2 views, amber dots, focused emphasis) + group frame (multi-session views only) + layout insets; `d`/`D`(picker overlay)/`Tab` bindings + help overlay; strip notices
- [x] 3.2 Auto-subtitle in tile bar (dim, ellipsis, lowest priority) + queue rows; model `title` plumbed from the API
- [x] 3.3 Mouse: view-strip click focuses a view; existing click/wheel behavior unchanged

## 4. Verification

- [x] 4.0 Daemon poller wave-2 fix: extend the per-tick diff to also compare each garage session's normalized pane title against the previous tick, emitting `sessions-changed` on a title-only change too (reuses the existing per-tick list-panes call — no new tmux invocation); unit tests (title change → event, unchanged → no event, session death still behaves)
- [x] 4.1 All existing wall/test/e2e suites green (no regressions) — `run_p81.sh`'s hardcoded PTY-size assertions updated for p10's always-on group framing (not a wall/src fix); one pre-existing, non-p10 flake found and reported, not fixed; recorded in verification.md
- [x] 4.2 New e2e run_p10.sh: detach/rejoin/cycle with frame+strip assertions, persistence across TUI restart, cross-view a-jump, subtitle render from a real OSC title; recorded in verification.md
