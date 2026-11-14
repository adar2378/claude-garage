## Why

The owner runs several projects at once. Each project has several git repos, and many Claude sessions (local and cloud) work through OpenSpec changes and GitHub pull requests. Today the state of that work is spread over four places: `openspec/changes/*/tasks.md` checkboxes in each repo, pull requests, CI and review comments on those pull requests, and the garage wall. There is no single glance that answers "what is happening, and what is waiting on me?".

Two things make it worse. An integration branch such as `feat/<name>` can hold a change that the local checkout does not have, so the local files lie. And the wall knows which session needs input, but not which change that session is building or which PR is waiting for approval.

garage already owns the right pieces: workspaces (project folders), sessions with statuses and branches, a wall with overlays, a loopback daemon API, and a plugin example (`examples/openspec-progress`) that already counts `tasks.md` checkboxes for the local checkout. This change turns that seed into a core, read-only **workspace overview**.

## What Changes

- **A workspace overview.** For each garage workspace it lists every OpenSpec change in the workspace's repos: state (`proposal`, `building`, `complete`), `done/total` tasks, the next open task, and where the change lives (local checkout, or a remote branch only).
- **Remote branches, read without touching the working tree.** After an opt-out `git fetch`, changes are read from `origin/<branch>` with `git ls-tree` and `git show`. The checkout is never modified.
- **Pull requests tied to changes and tasks.** Matched by branch name or a title prefix `[<change> <task>]`. Each PR shows the latest check result and review verdict when the repo posts them as PR comments (configurable regexes), and whether that verdict is for the PR's current head commit.
- **A "waiting on you" list.** Proposals to approve, PRs whose review says owner approval is needed, failed checks on the current head, sessions in `needs-input`.
- **Sessions linked to changes** by worktree branch or session label.
- **Three surfaces over one daemon endpoint.** A wall overlay (key `o`, `j`/`k`, `Enter` jumps to the session, `p` opens the PR), a CLI command `claude-garage tasks`, and `GET /api/overview` for the web and marketing site.
- **Declared per workspace** in `~/.garage/overview.toml` (repos, branch patterns, PR and comment patterns). With no file, garage scans the workspace folder and its direct children for git repos with an `openspec/` directory.
- **GitHub through the `gh` CLI already on the machine**, REST only (`gh api`), cached with ETags, rate-limit aware, and silent when offline. Off until a workspace opts in, so the README's "100% local" promise stays true by default.
- **Read-only.** The overview never approves, merges, comments, edits `tasks.md` or starts sessions.

## Capabilities

### New Capabilities
- `workspace-overview`: discovery, OpenSpec change reading (local and remote), PR and comment linking, the waiting list, session links, configuration, the wall overlay and the `tasks` command.

### Modified Capabilities
- `daemon-api`: `GET /api/overview`, `POST /api/overview/refresh` and the `overview` event.

## Delivery order (owner priority first)

1. **Peek view of one workspace's changes and PR status** (tasks 1 to 7). Usable on its own as `claude-garage tasks` and the `o` overlay.
2. **Waiting on you and session links** (tasks 8 to 10).
3. **Web/site endpoint and refresh polish** (tasks 11 and 12).
4. **Docs, README privacy text** (task 13).

Each group ships alone; later groups only add rows and columns.

## Open questions for the owner

Each has a default. If you do not answer, the default is built.

1. **Where does the config live?** Default: one file, `~/.garage/overview.toml`, with a `[workspace.<name>]` table per workspace, because a project's repos do not share a parent repo. A committed per-repo `.garage/overview.toml` (patterns only, no paths) can be added later.
2. **Is GitHub access on by default?** Default: **off** per workspace until it has `github = true`. First use prints one line saying that garage will run `gh api` against github.com for that workspace's repos. Alternative: on whenever `gh` is logged in.
3. **May garage `git fetch`?** Default: yes, `git fetch --prune origin` (remote-tracking refs only, never the working tree), every 5 minutes while someone is looking at the overview, and on `r`. Alternative: never fetch, read only what is already fetched.
4. **What counts as "awaiting approval"?** Default: a change in state `proposal` (no task ticked) that has no merged PR. Alternative: require a label such as `proposal` on the PR.
5. **Key for the overlay.** Default: `o` (free today; `n N x X r P p ? q w d D a A m t b R I u` are taken). Alternative: `g`.
6. **Which remote branches are scanned?** Default: remote branches whose `openspec/changes` tree differs from the default branch and that had a commit in the last 30 days. Alternative: only branches named in the config.
7. **Retire `examples/openspec-progress`?** Default: keep it; the overview reuses its checkbox counting rules and the example stays a reference plugin.
8. **Can the site read it?** Default: the endpoint follows the daemon's existing rule (`GARAGE_UI_ORIGINS`). Nothing is exposed beyond `127.0.0.1`.

## Impact

- **Code:** new crate `crates/overview` (pure collector and parsers, no UI); daemon module `crates/daemon/src/api/overview.rs` (cache, polling, SSE); `crates/cli` gets a core command `tasks`; the wall gets an overlay in `wall/src/ui/overview.rs` that reads only the daemon API.
- **Docs:** README (keys, Privacy wording), `docs/` page for `overview.toml`, `docs/plugins/cli.md` (reserved command word `tasks`).
- **New runtime dependencies on the machine:** `git` (already required) and optionally `gh`.
- **No new network destination** other than github.com through the user's own `gh` login, and only when a workspace opts in. No telemetry, no account, no server.

## Out of scope

Approving or merging a PR; editing `tasks.md`; writing PR comments; starting or steering sessions from the overview; GitLab or other hosts; webhooks (polling only); notifications for overview events (the existing `alerts` plugin keeps handling `needs-input`); a cross-workspace dashboard page on the site (the endpoint only); reading non-OpenSpec task lists.
