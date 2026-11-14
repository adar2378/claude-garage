## Why

The owner runs several projects at once. Each project has several git repos, and many Claude sessions work through OpenSpec changes and GitHub pull requests. Today the state of that work is spread over four places: `openspec/changes/*/tasks.md` checkboxes in each repo, pull requests, CI and review comments on those pull requests, and the garage wall. There is no single glance that answers "what is happening, and what is waiting on me?".

Two things make it worse. An integration branch such as `feat/<name>` can hold a change that the local checkout does not have, so the local files lie. And the wall knows which session needs input, but not which change that session is building or which PR is waiting for approval.

The owner decided (15 Nov 2026) that this is **not a core feature**. garage core stays small; the overview is a plugin, opt-in, outside `plugins/`, with no core change at all. The plugin API v1 already has what it needs: an overlay pane, a status-strip widget, session and workspace reads, navigation actions and event hooks. The existing `examples/openspec-progress` plugin is the seed and stays.

Sessions here means **garage sessions** (what `GET /api/sessions` returns). Cloud sessions are not garage sessions and are out of scope; a cloud session's work shows up only through its branch and pull request.

## What Changes

- **A plugin `openspec-board`** in `extras/openspec-board/`: a Rust crate on the public SDK with its own `[workspace]` table (like `examples/openspec-progress`), so it is outside the garage workspace, outside `plugins/`, not embedded in the `claude-garage` binary and not allowlisted. **No core change.** Install: `claude-garage plugin install adar2378/claude-garage/extras/openspec-board`. The install preview shows its permissions (`sessions.read`, `ui.widgets`, `ui.panes`, `ui.invoke`, `ui.navigate`; none dangerous).
- **An overlay pane** (`openspec-board:board`, suggested key `o`, never overrides a bound key) listing, for the focused workspace, every OpenSpec change: state (`proposal`, `building`, `complete`), `done/total` tasks, next open task, and where it lives (local checkout or remote branch only). This peek pane is task group 1, the owner's first priority.
- **Remote branches, read without touching the working tree**: `git fetch` (remote-tracking refs only, flags in the design), then `git ls-tree` and `git show` on `origin/<branch>`. Fetch runs only while the overlay is open and on `r`. Fetch talks only to the user's own `origin`.
- **Pull requests tied to changes and tasks** through `gh api` (REST only, ETag cached, rate-limit aware, silent offline), off by default per workspace, with CI and review verdicts read from PR comments by configurable regexes, marked current or stale against the PR head. A change is linked to a PR when its name appears anywhere in the PR's branch, title or diff paths.
- **A "waiting on you" list**: proposals to approve (an open PR whose diff adds the change folder, so GitHub must be on), owner-review verdicts, failed checks on the current head, sessions in `needs-input`.
- **Sessions linked to changes**, read from the daemon's `GET /api/sessions`. `Enter` jumps to the session through `POST /api/plugins/ui/action` (`session.focus`); `p` opens the PR in the browser from the plugin's own process.
- **A status-strip widget** (`strip.right`) with the "waiting on you" count, from live sessions plus the last cached model, marked stale when that model is old. Clicking it opens the pane.
- **A `claude-garage tasks` plugin CLI command** printing the same rows as text or JSON.
- **Declared per workspace in the plugin's own config**: `<state dir>/config.toml`, written by the user, never by the plugin. It lives in the plugin's state folder, so `plugin remove --purge` deletes it. Cache lives beside it in `cache/` and is disposable but private: it holds PR titles and comment bodies.
- **Build-from-source install only for now.** No plugin release tags and no release assets (see the design: any tag with `X.Y.Z` triggers garage's own cargo-dist release and `scripts/check-release-tag.sh`). A release path is later work.
- **Read-only.** The plugin never approves, merges, comments, edits `tasks.md`, starts or types into a session. It asks for no dangerous permission.

## Capabilities

### New Capabilities
- `workspace-overview`: what the `openspec-board` plugin shows and does: discovery, OpenSpec reading (local and remote), PR and comment linking, the waiting list, session links, configuration, pane, strip widget and `tasks` command.

### Modified Capabilities
None. No core change, so the earlier `first-party-plugins` delta is dropped.

## Decided (owner, 15 Nov 2026)

The owner's standing instruction is to take the recommended defaults, so these are decided, not open:

1. **Location and install.** `extras/openspec-board/` with its own `[workspace]` (like `examples/openspec-progress`), outside `plugins/`, so no core change (the former C1 is dropped). `claude-garage plugin install adar2378/claude-garage/extras/openspec-board`. Opt-in, never seeded or bundled. The peek pane is group 1.
2. **Release.** Build-from-source install only for now (`[[build]]` with `cargo build --release`, commands pointing at the real `target/release` path as the example does). No plugin release tags. A release path is noted as later work.
3. **Strip widget colour.** The `needs_input` token only when sessions are waiting on input; `warn` for proposals and failed CI (plugins cannot paint amber otherwise). The count is shown as stale when the cached model is old.
4. **"Awaiting approval".** An open PR whose diff adds the change folder. With GitHub off there are no approve-proposal items. Change names already in the default branch's `archive/` are skipped. Linking: the change name matched anywhere in the PR's branch, title or diff paths, so `claude/<anything>` branches and `docs(openspec): propose <change>` titles link. No first-token branch regex.
5. **Fetch.** On, but only while the overlay is open or on `r`, with `--no-auto-gc --no-auto-maintenance --no-write-fetch-head`, null stdin and `GIT_SSH_COMMAND='ssh -o BatchMode=yes'`. It writes nothing outside `refs/remotes` (plus the object store every fetch fills) and talks only to the user's own remote. GitHub is off by default per workspace.
6. **Config, key, other.** Config in the plugin's state folder, like `alerts` (lost on `plugin remove --purge`). Key `o`. `examples/openspec-progress` is kept. No marketing-site route (the former C2 and group 12 are dropped). Opt-in install.

## Delivery order (owner priority first)

1. **Group 1: the read-only peek pane**, local changes: skeleton, parsers, config, collector, pane.
2. **Groups 2 and 3**: remote branches, then PR status. Usable on their own.
3. **Group 4** waiting list, session links, jump and open-PR; **group 5** strip widget; **group 6** `tasks` command.
4. **Group 7**: build, README, privacy note, CI for the crate.

Each group ships alone, and each group adds to the manifest only what it implements (hooks, commands and permissions are never declared ahead).

## Impact

- **New code:** `extras/openspec-board/` only (manifest, Rust crate on `garage-plugin-sdk`, tests, README). Its own `Cargo.toml` with its own version (`0.1.0`), its own `Cargo.lock`, its own `[workspace]`.
- **Core:** none. No new route, event, permission, wall change or CLI word; no change to `crates/daemon/build.rs` or the garage workspace.
- **Docs:** `extras/openspec-board/README.md` (config reference, keys, privacy), a line in `docs/plugins/README.md`, README "Privacy" wording for the plugin.
- **New runtime dependencies on the machine:** `git` (already required), `cargo` to build the plugin, and optionally `gh`.
- **No new network destination** other than the user's own `origin` (fetch) and github.com through the user's own `gh` login, and the latter only when a workspace opts in. No telemetry, no account, no server.

## Out of scope

Approving or merging a PR; editing `tasks.md`; writing PR comments; starting or steering sessions; cloud sessions (they are not garage sessions); GitLab or other hosts; webhooks (polling only); notifications for overview events (the `alerts` plugin keeps handling `needs-input`); a marketing-site route or cross-workspace dashboard page; reading non-OpenSpec task lists; changes to the wall, the daemon routes or the plugin API; plugin release tags and prebuilt binaries (later); making `openspec-board` part of the core feature set.
