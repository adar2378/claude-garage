## Why

The owner runs several projects at once. Each project has several git repos, and many Claude sessions (local and cloud) work through OpenSpec changes and GitHub pull requests. Today the state of that work is spread over four places: `openspec/changes/*/tasks.md` checkboxes in each repo, pull requests, CI and review comments on those pull requests, and the garage wall. There is no single glance that answers "what is happening, and what is waiting on me?".

Two things make it worse. An integration branch such as `feat/<name>` can hold a change that the local checkout does not have, so the local files lie. And the wall knows which session needs input, but not which change that session is building or which PR is waiting for approval.

The owner decided (15 Nov 2026) that this is **not a core feature**. garage core stays small; the overview is a first-party **plugin**, opt-in, shipped and updated on its own. The plugin API v1 already has what it needs: an overlay pane, a status-strip widget, session and workspace reads, navigation actions and event hooks. The existing `examples/openspec-progress` plugin is the seed.

## What Changes

- **A first-party plugin `openspec-board`** in `plugins/openspec-board/`, a Rust program on the public SDK. It is not bundled into the `claude-garage` binary and not enabled by default: users add it with `claude-garage plugin install adar2378/claude-garage/plugins/openspec-board`, see its permissions in the install preview, and update it with `claude-garage plugin update` on its own release cadence.
- **An overlay pane** (`openspec-board:board`, suggested key `o`, never overrides a bound key) listing, for the focused workspace, every OpenSpec change: state (`proposal`, `building`, `complete`), `done/total` tasks, next open task, and where it lives (local checkout or remote branch only).
- **Remote branches, read without touching the working tree**: `git fetch --prune origin`, then `git ls-tree` and `git show` on `origin/<branch>`.
- **Pull requests tied to changes and tasks** through `gh api` (REST only, ETag cached, rate-limit aware, silent offline), with CI and review verdicts read from PR comments by configurable regexes, marked current or stale against the PR head.
- **A "waiting on you" list**: proposals to approve, owner-review verdicts, failed checks on the current head, sessions in `needs-input`.
- **Sessions linked to changes**, read from the daemon's `GET /api/sessions`. `Enter` jumps to the session through `POST /api/plugins/ui/action` (`session.focus`); `p` opens the PR in the browser from the plugin's own process.
- **A status-strip widget** (`strip.right`) showing the "waiting on you" count, amber only when something needs the human. Clicking it opens the pane.
- **A `claude-garage tasks` plugin CLI command** printing the same rows as text or JSON.
- **Declared per workspace in the plugin's own config**: `<state dir>/config.toml` (`~/.garage/plugins/data/openspec-board/config.toml`), written by the user, never by the plugin. Cache lives beside it in `cache/` and is disposable. With no config, the plugin scans each workspace folder and its direct children.
- **Read-only.** The plugin never approves, merges, comments, edits `tasks.md`, starts or types into a session. It asks for no dangerous permission.
- **One small, separate core change** (group 1 of the tasks): first-party folders under `plugins/` are embedded and allowlisted automatically by `crates/daemon/build.rs`; the daemon needs a short list of first-party plugins that are shipped separately, so `openspec-board` is not embedded. Nothing else in core changes. Section "Core changes" of the design lists everything checked and why no other core change is needed.

## Capabilities

### New Capabilities
- `workspace-overview`: what the `openspec-board` plugin shows and does: discovery, OpenSpec reading (local and remote), PR and comment linking, the waiting list, session links, configuration, pane, strip widget and `tasks` command.

### Modified Capabilities
- `first-party-plugins`: a first-party plugin may be shipped separately from the `claude-garage` binary (opt-in, own releases, not embedded, not allowlisted).

## Delivery order (owner priority first)

1. **Core change** so the plugin folder is not embedded (task group 1). Lands before the plugin folder exists.
2. **Read-only peek pane**, local changes first (groups 2 to 4), then remote branches (5) and PR status (6). Usable on its own.
3. **Waiting on you, session links, jump and open-PR** (group 7), **strip widget** (8), **`tasks` command** (9).
4. **Release pipeline and docs** (groups 10 and 11).
5. **Optional, off by default**: a core read route for the marketing site (group 12), only if the owner answers yes to question 8.

Each group ships alone; later groups only add rows and columns.

## Open questions for the owner

Each has a default. If you do not answer, the default is built.

1. **Where does the config live?** Default: the plugin's own state folder, `~/.garage/plugins/data/openspec-board/config.toml`, with a `[workspace.<name>]` table per workspace, because a project's repos do not share a parent repo and plugin conventions give each plugin one writable folder. The plugin only reads this file. Alternative: `~/.garage/openspec-board.toml`, outside the state folder (survives `plugin remove --purge`).
2. **Is GitHub access on by default?** Default: **off** per workspace until it has `github = true`. First use prints one line saying that the plugin will run `gh api` against github.com for that workspace's repos. Alternative: on whenever `gh` is logged in.
3. **May the plugin `git fetch`?** Default: yes, `git fetch --prune origin` (remote-tracking refs only, never the working tree), every 5 minutes while the pane is open, and on `r`. Alternative: never fetch, read only what is already fetched.
4. **What counts as "awaiting approval"?** Default: a change in state `proposal` (no task ticked) that has no merged PR. Alternative: require a label such as `proposal` on the PR.
5. **Key for the pane.** Default: suggest `o` (free today; `n N x X r P p ? q w d D a A m t b R I u /` are taken). A suggestion never overrides an existing binding; users can rebind with `plugin bind`. Alternative: `g`.
6. **Which remote branches are scanned?** Default: remote branches whose `openspec/changes` tree differs from the default branch and that had a commit in the last 30 days. Alternative: only branches named in the config.
7. **Retire `examples/openspec-progress`?** Default: keep it as the small reference plugin; `openspec-board` reuses its checkbox rules by copy and supersedes it for real use. Alternative: remove the example once `openspec-board` ships.
8. **Should the marketing site be able to read the overview?** Default: **no**. A plugin cannot add daemon routes, and the site is not part of the owner's priority list. If yes, group 12 adds a small generic core route that serves a JSON blob a plugin published; the plugin would publish its model there.
9. **Is the plugin enabled for everyone on install?** Default: opt-in only, never seeded. It is installed with `plugin install`, nothing ships it by default. Alternative: bundle it but off by default (then it updates only with garage releases and group 1 is not needed).

## Impact

- **New code:** `plugins/openspec-board/` (manifest, Rust crate on `garage-plugin-sdk` with the `tui` feature where it helps, tests). No new crate in core.
- **Core:** only the small embed-exclusion list in `crates/daemon/build.rs` and its tests (group 1). No new route, no new event, no new permission, no wall change, no new CLI word in core.
- **Release:** a plugin release workflow publishing the binaries and a checksums file (group 10), using the manifest `[release]` table.
- **Docs:** `plugins/openspec-board/README.md` (config reference, keys, privacy), a line in `docs/plugins/README.md` listing it, README "Privacy" wording for the plugin.
- **New runtime dependencies on the machine:** `git` (already required) and optionally `gh`.
- **No new network destination** other than github.com through the user's own `gh` login, and only when a workspace opts in. No telemetry, no account, no server. Idle cost is one tiny process a minute for the widget, which reads a few local files and one local HTTP call.

## Out of scope

Approving or merging a PR; editing `tasks.md`; writing PR comments; starting or steering sessions; GitLab or other hosts; webhooks (polling only); notifications for overview events (the `alerts` plugin keeps handling `needs-input`); a cross-workspace dashboard page on the site; reading non-OpenSpec task lists; changes to the wall, the daemon routes or the plugin API (apart from the one optional group 12); making `openspec-board` part of the core feature set.
