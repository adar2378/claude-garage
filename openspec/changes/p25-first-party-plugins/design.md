> **Detailed design is written when the phase starts (after p21/p22).** This file records only the decisions already made.

## Decisions made

**1. The acceptance test.** Every "plugin" row in the proposal's map runs as a plugin through the public plugin API only, with no private hooks. If a plugin needs something the API lacks, the gap is added to the plugin host (p24), not worked around.

**2. Core owns data and lifecycle; plugins own opinions and presentation.** Example: the daemon keeps closed-session records (data), the closed-chats plugin decides how to show them.

**3. Core items stay core.** Draft and screenshot recovery needs to read every keystroke sent to a session; without a sandbox that is too much trust to hand a plugin. Selection, copy and sidebar layout are terminal fundamentals.

**4. First-party plugins live in the repo** (`plugins/<name>/`), ship with garage and are enabled by default where the feature existed before (pit pet). They use the same manifest and install path as third-party plugins.

**5. Freeze rule.** Plugin API v1 is frozen only when every plugin here works on macOS and Linux. Freezing earlier would freeze guesses.

## Open for the phase start
- Which plugins are enabled by default.
- Whether turn markers need a new scrollback slot or can use widgets.
- Source of truth for the usage figures (statusline only, or transcript fallback).

## Resolved (2026-10-31, user: all defaults)

- On by default: usage bar, closed chats, quick send, close/delete buttons, pet. Off by default: jump palette, turn markers.
- Two plugins in one slot: both render, in install order; a config setting can reorder them.

## Detailed design: closed-chats, jump-palette, quick-send, tile-buttons, prune-empty-projects, cli-extras

Written at the start of the p25 build (the other plugins, usage-bar, turn-markers and pit-pet, are designed in their own section). All six are bundled Rust plugins in `plugins/<name>/` (binary name = directory name, which `scripts/stage-plugins.sh` and `release.yml` rely on), built on `crates/plugin-sdk`. They use only the public API: manifest, run context, plugin token routes and the existing core HTTP routes.

### Shared pieces

**SDK growth, only what these plugins need.** `Api::get / post / put_json / delete` (thin over `request`), `Api::ui_action(action, target)`, `Api::session_input(id, text)`, a typed `Context` view over the run-context JSON (focus workspace and session, event name and data, CLI argv, cwd), and a `tui` cargo feature with a small raw-mode terminal toolkit for pane plugins: a fuzzy list picker and a one-line prompt. Both are pure state machines (`Picker::on_key`, `Prompt::on_key` return an outcome) with a thin crossterm renderer around them, so the logic is unit-tested without a terminal. The feature is off by default so plugins without a pane do not link crossterm. A `fuzzy` function (subsequence match, scored: prefix and word-start bonuses, shorter is better) is part of the feature.

**Pane lifetime.** A pane is a normal process in a PTY. The plugin reads its context from `GARAGE_CONTEXT`, draws, acts, and exits. The host closes an overlay or popup when the process exits, so "choose, run, exit" is the whole protocol. Esc or Ctrl-C exits with no action. The pane never reads keys outside its own PTY.

**Manifest-only plugins.** `tile-buttons` has no commands. The cargo `plugins/*` member glob refuses a directory with no `Cargo.toml`, so such plugins are listed in the workspace `exclude`; `stage-plugins.sh` and the release check skip the binary when the plugin has no crate.

**Error handling.** A plugin run that cannot do its job prints one line to stderr and exits non-zero; the host toasts "plugin: action failed, see `plugin logs`". Event hooks never retry beyond the small bounded loops named below.

### closed-chats (2.1), on by default

Permissions: `ui.widgets`, `sessions.read`, `core.actions`, `workspaces.read`. Widget `rows` in `rail.extra_rows`; action `recent` with a `recent` overlay pane and `default_key = "H"`.

- **Rows.** A `sync` run lists workspaces (`GET /api/workspaces`), fetches each one's records (`GET /api/workspaces/{name}/closed`), and PUTs one widget with a scope per workspace. The rail allows 8 extra rows per workspace and the daemon keeps 20 records, so the newest 8 show. Each row has `session = <closed session id>` so the click targets it (not the focused session), dim text (`fg: dim`, no background), and two clickable parts: the label (`click: session.restore`) and a trailing ` x ` (`click: session.delete`, a dangerous core action, so it goes through the confirm gate). `session.restore` is not dangerous. The label is `<label>` plus a reason hint (`closed`, `crashed`, `restart`) in dim.
- **When `sync` runs.** Event hooks: `session.closed`, `session.created` (a restore or new chat drops its record), `session.forgotten`, `workspace.added`, `workspace.removed`, `daemon.started` (widgets are in memory and lost on daemon restart). Two gaps found and closed in the host: forgetting a record (`DELETE /api/sessions/{id}?meta=1`) published no plugin-visible event, so a deleted row would stay, and it answered 404 for a deliberately closed session (only its closed record exists, the resume metadata was removed with the close), so the row's `x` could never work. The route now forgets a closed-only record and publishes `session.forgotten {id, workspace}` (additive event, listed in `/api/plugin/features`). See tasks 4.1.
- **Race.** The closed record is written by a bus subscriber, concurrently with the event hook starting. On `session.closed` the run retries the fetch up to 5 times, 150 ms apart, until the record for the event's session id appears.
- **`H` recent list.** An overlay pane showing every workspace's records (up to 20 each, newest first) in a picker: `workspace/label  reason  age`. Enter restores via `POST /api/sessions/restore {id}` (a core route) and exits; `d` is not offered (forget goes through the rail's confirmed delete). With nothing closed the pane shows "no closed chats" and exits on any key.
- **Tests.** Row building (limit 8, newest first, session and click fields, reason hints); picker item formatting and the age text; retry loop against a fake HTTP server; daemon end-to-end: the real binary behind the real host, a `SessionClosed` on the bus, the `rows` widget appears with the restore click.

### jump-palette (2.2), off by default

Permissions: `sessions.read`, `workspaces.read`, `ui.invoke`, `core.actions`, `ui.panes`. Action `open` (pane `palette`, overlay), `default_key = "/"`.

The pane fetches `GET /api/sessions` (live sessions only; `restorable` entries are skipped) and `GET /api/workspaces`, builds items (`workspace/label  status  branch` for sessions, `workspace  dir` for workspaces, sessions first) and shows the fuzzy picker. Enter calls `POST /api/plugins/ui/action` with `session.focus` and `target.session`, or `workspace.focus` with `target.workspace`, then exits. `session.engage` is refused for plugins by design, so the palette moves focus but never starts typing into a session. The fuzzy query matches against the whole item text.

Tests: `fuzzy` ranking table (prefix beats mid-word, subsequence required, case-insensitive, stable ties); item building from a canned session list; ui/action body shape against a fake server; picker key handling.

### quick-send (2.3), on by default

Permissions: `sessions.input` (DANGEROUS), `sessions.read`, `ui.panes`. Actions `send` (context `session`, pane `send`, `default_key = "s"`) and `send-all` (context `workspace`, pane `send-all`, `default_key = "S"`); both panes are popups.

The text must come from the user, so each action opens a popup that shows the target ("send to garage/app/main" or "send to 3 sessions in app"), a one-line input, and is its own confirmation step: Enter on a non-empty line moves to "Enter again to send, Esc to cancel", the second Enter sends. Nothing is sent without two human key presses in the pane, so the manifest's `confirm = true` (which gates plugin command runs, and for pane actions is not applied by the wall's armed state) is not what protects it. The `S` targets are the live sessions whose id starts with `garage/<workspace>/` from `GET /api/sessions`; each gets `POST /api/sessions/input {id, text}`. A failure on one target is reported in the popup and the others still go. The popup shows the result for a moment (a failed send waits for a key) and exits.

Tests: prompt state machine (typing, backspace, empty line refused, two-step confirm, Esc cancels at both stages); target resolution for one and for all; per-target failure reporting against a fake server.

### tile-buttons (2.4), on by default

Manifest only. Permissions: `core.actions`. Two `[[buttons]]` in `tile.title.right`, both `confirm = true`: ` x ` (`session.close`, priority 5) and ` del ` (`session.delete`, priority 4, dropped first on a narrow tile). Both actions are dangerous, so a click only arms them ("Close garage/app/main? click again or press y"); the host's confirm rule is the whole safety story and is covered by the wall tests from p24.

**Delete on a live tile.** `session.delete` used to forget only a closed session, and the wall refused it on a live tile, so a tile button could never act. The host now has the path it needed: `DELETE /api/sessions/{id}?forget=1` kills the live session and keeps no closed record and no resume data (a closing marker tells the poller to stay silent, so the recorder never writes one), and publishes `session.forgotten`. The wall's `session.delete` on a live tile calls it, and its arm prompt says there is no way back: "Delete ws/label? (kills it, no history)". The closed-chats rail row keeps its own ` x `, which forgets a closed record (`?meta=1`) as before. Close and delete differ on purpose: close leaves a grey row that restores; delete leaves nothing.

The tests: the manifest parses, validates, is on the compiled allowlist and seeds as enabled with both buttons asking first; the daemon test for `?forget=1`; the wall test for the arm prompt and the effect; and a wall client test for the request.

### prune-empty-projects (2.7), off by default

Permissions: `sessions.read`, `workspaces.write`. Event `session.closed`.

On an event whose `data.reason` is `user`, wait 1.5 s (a user who closes a chat and opens another a moment later must not lose the project), then read `GET /api/sessions`. "Empty" means no entry with `workspace == data.workspace` at all: not live, and not `restorable: true` (a restorable entry is a crash waiting to be resumed). If empty, `DELETE /api/workspaces/{name}`; 404 counts as success (closing many sessions at once races several events against one delete). `crash` and `daemon-restart` never prune. The workspace delete also drops the workspace's closed records (the core's `remove_workspace`), so with closed-chats enabled a pruned project's closed chats disappear with it; the two are independent and the docs say so.

Tests: the decision function as a table (reason user/crash/daemon-restart, live and restorable entries, other workspaces' sessions); 404 tolerated; daemon end-to-end with the real binary and a fake `session.closed`.

### cli-extras (2.8), off by default

Permissions: `workspaces.read`, `workspaces.write`, `sessions.read`, `sessions.manage`. Three `[[cli]]` entries: `install`, `prune`, and `open` with `fallback = true`. The fallback receives the whole argv, so `claude-garage ~/dev/app` and `claude-garage open ~/dev/app` both work.

- **`<folder>`**. Resolves the word to a directory (`~`, `~user`, relative paths against the cwd, canonicalised; a path that is not UTF-8 is refused). The launcher does this check itself before it starts or replaces any daemon: if the word is not an existing directory it prints `unrecognized subcommand '<word>'` and exits 2 without touching the daemon, because a fallback otherwise swallows every typo, which would be the worst UX in the product. Workspace name: the directory's basename, lowercased, every run of characters outside `[a-z0-9]` collapsed to `-`, trimmed of `-`; empty is an error. If a workspace with that name exists for a different directory, `-2`, `-3`, ... are tried; if it exists for the same directory it is reused. Then `PUT /api/workspaces {name, dir}` (skipped when the folder is already a project under any name, which is reused). If the project has no live session it gets `POST /api/sessions {workspace, label}` with the first free `claude-N` label, the wall's own default family (a 409, somebody else started it, is fine); a project that already has sessions gets no second one. It then asks for the wall (skipped by `--no-open`): it writes `{"open_wall": true}` to the file named by `$GARAGE_HANDOVER` and exits 0, and the launcher runs the wall itself, in its own environment, so the plugin's token and run context never reach it. Registered folders are compared by canonical path on both sides.
- **`install`**: `POST /api/hooks/install` then `POST /api/statusline/install`; prints what each did; non-zero if either failed.
- **`prune`**: removes every workspace with no entry at all in `GET /api/sessions` (the same "empty" as above); `--dry-run` lists without deleting. Prints each removed name.
- **Daemon not running.** No launcher logic lives in a plugin. All three `[[cli]]` entries set the new `needs_daemon = true`, and `claude-garage` then runs the launcher's daemon check (start one if none, replace a stale one) before the plugin command starts, with the same messages as the TUI launcher. See tasks 4.1.

Tests: name derivation table; directory resolution (tilde, relative, not-a-dir fallthrough); the folder flow's HTTP calls against a fake server (including the 409 and the name-collision paths); prune selection; install result handling; CLI end-to-end: the real `claude-garage plugin` dispatch to the fallback is already covered by p24 (6.2), here the plugin's own binary runs against a fake daemon.

### Allowlist and packaging

`build.rs` hashes every `plugins/*/garage-plugin.toml` except `hello`, so each manifest above lands on the compiled allowlist by existing. The daemon tests assert that each id is on the list and that its manifest validates (`bundled.rs` already walks the folder). `DEFAULT_ON` is not touched: closed-chats, quick-send and tile-buttons are on, jump-palette, prune-empty-projects and cli-extras are off.
