# Tasks: p25-first-party-plugins

## 1. Core data
- [x] 1.1 Daemon keeps closed-session records (id, workspace, title, closed at) with an API to list them (records also keep the worktree, so a restored worktree chat can still be finished)

## 2. First-party plugins (`plugins/`)
- [x] 2.1 closed-chats (rail widget rows + `H` recent overlay)
- [x] 2.2 jump-palette (`/` overlay pane)
- [x] 2.3 quick-send (`s` / `S`)
- [x] 2.4 tile-buttons (close button; the delete button is blocked by an open gap, see 4.1)
- [ ] 2.5 usage-bar (5h / weekly, persisted)
- [ ] 2.6 turn-markers
- [x] 2.7 prune-empty-projects (`session.closed` hook)
- [x] 2.8 cli-extras (`<folder>`, `install`, `prune`)
- [ ] 2.9 pit-pet (moved out of `ui/pet.rs`, same behaviour)

## 3. Core items
- [ ] 3.1 Draft and pasted-screenshot recovery
- [ ] 3.2 Mouse selection and copy
- [ ] 3.3 Sidebar hide and resize
- [ ] 3.4 Quiet startup (`GARAGE_VERBOSE=1` for full output)

## 4. API freeze
- [ ] 4.1 List every plugin API gap found in group 2 and close it in the plugin host
  Gaps found by closed-chats, jump-palette, quick-send, tile-buttons, prune-empty-projects and cli-extras (the other plugins add theirs below). Closed in the host:
  - [x] `DELETE /api/sessions/{id}?meta=1` published no plugin-visible event, so a forgotten closed chat stayed in the rail: new additive event `session.forgotten {id, workspace}` (manifest enum, features list, schema, SSE frame).
  - [x] The same route answered 404 for a deliberately closed session (a close removes the resume metadata, only the closed record is left), so `session.delete` on a closed-chat row could never work: it now forgets a closed-only record.
  - [x] The `plugins/*` workspace glob refuses a plugin directory with no `Cargo.toml`, and `stage-plugins.sh` / `release.yml` demanded `bin/<id>` for every plugin: manifest-only plugins (tile-buttons) are `exclude`d from the workspace and packaged as manifest only.
  - [x] A `[[cli]]` command that calls the daemon API failed when no daemon was running (`claude-garage ~/dev/app` on a cold start): new `[[cli]] needs_daemon = true`; `claude-garage` runs the launcher's start-or-replace daemon check first (`launcher::ensure_daemon`).
  - [x] A CLI plugin could not hand over to the wall (it does not know where `claude-garage` is): CLI runs now get `GARAGE_BIN`.
  - [x] The SDK had no way to call core routes, ask the wall to focus something, type into a session, read the run context in a typed way, fuzzy-match, draw a picker or prompt in a pane, or fake the daemon in tests: `Api::get/post/put/delete/ui_action/session_input`, `Context`, `fuzzy`, `tui` (feature) and `testing` (feature).
  Open (not closed here):
  - [ ] A tile delete button cannot work: `session.delete` forgets a closed session and the wall refuses it on a live tile ("close it first"). Needs a "close and forget" path: daemon `DELETE /api/sessions/{id}?forget=1` (kill without leaving a closed record; today both the handler and the `session.closed` subscriber write one) and a wall action that uses it; then tile-buttons gets its second button. Needs a decision on the semantics (does forgetting also drop a worktree session's resume data?).
  Also closed: a CLI plugin's own `claude-garage` child (cli-extras auto-open) found the wrong daemon under a custom `GARAGE_PORT` / `GARAGE_DIR` because CLI runs get a scrubbed environment: those settings (`GARAGE_PORT`, `GARAGE_DIR`, `GARAGE_TMUX_SOCKET`, `GARAGE_TMUX_CONTROL`, `GARAGE_BACKEND`) are now passed through.
  Noted, not changed:
  - A manifest `confirm = true` on an action applies to plugin-origin triggers (clicks, `ui/action`) only; a key press runs the action. quick-send therefore confirms inside its popup (Enter twice) instead of using it.
  - A plugin cannot read focus changes, so jump-palette cannot highlight the current session; it lists them all.
  - Widgets are in memory: closed-chats re-syncs on `daemon.started`. Persisting widgets across a daemon restart stays a v1 non-goal.
  - Rail limit: 8 extra rows per workspace, so a workspace shows its 8 newest closed chats; `H` lists up to the 20 kept per workspace.
- [ ] 4.2 Freeze plugin API v1 and document it

## 5. Verification
- [ ] 5.1 Unit and contract tests per plugin
- [ ] 5.2 Live: each plugin on macOS and Linux; pit pet behaves as before
- [ ] 5.3 Record results in `verification.md`
