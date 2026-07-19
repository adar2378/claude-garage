# Backlog — accepted, not yet scheduled

- **`claude-garage attach` (adopt the current tmux session)** — CLI subcommand run from inside any tmux session: renames it to `garage/<workspace>/<label>` and PUTs the workspace registration (name/dir derived from the session's cwd, flags to override), so an existing session joins the pit wall without respawning. Today's workaround: `tmux rename-session "garage/<ws>/<label>"` (appears next poll tick; unregistered `?` group unless the workspace is registered). Non-tmux terminals can't be adopted — a live process can't move into tmux; document `claude --resume` into a fresh garage session as the path for those. Needs bin/garage.js to grow subcommand parsing (currently start-only). (Requested 2026-07-19.)

- **grid-views follow-ups** (core shipped 2026-07-19 as `grid-views` — detach/rejoin, view strip, rail tier, per-view layouts, cross-view `a`-jump): view renaming; drag a session row between views in the rail; needs-first ordering recursing through the view tier in lib/groups.js (currently views keep creation order, main first).

- **p6 candidate: worktree sessions (opt-in)** — spawn-time "in worktree" toggle on the per-workspace `+` (default off, last state remembered per workspace): `git worktree add .garage/worktrees/<label> -b garage/<label>` and claude starts there. Close gains merge-or-discard for the branch; diff pane needs a per-session mode for worktree sessions; branch chips make them self-documenting in the rail. Decision 2026-07-19: configurable, never always-on — editing sessions want isolation, question/review sessions want zero friction, and forced worktrees import the merge-back problem into every spawn.

- **p5 candidate: session reset ("start fresh")** — per-session control that kills the running claude agent and starts a fresh conversation in the SAME tmux session, via `tmux respawn-window -k -t <id> claude` (session entity survives → attached clients stay connected, no WS reconnect). Daemon: `POST /api/sessions/reset {id}` + drop resume metadata so reboot-restore can't resurrect the pre-reset conversation. UI: ↺ control in cell title bar + rail row, two-click confirm ("really?" for ~3s). Old conversation remains reachable via claude's own `--resume` picker. (Requested 2026-07-19, queued behind p4.)

- Folder-trust dialog status blind spot: fresh sessions in never-trusted dirs block invisibly (no hook, agents-json says idle). Candidate: treat a young session with no signals as needs-attention. (Found during P1 e2e.)

- `garage deck <workspace>`: tmux-native tiled layout as terminal-first pit-wall-lite. (Deferred surface decision, IDEA.md.)

- npm publish: package still `private: true`; needs user's npm account + name claim.

- Linux: notifications are a darwin-only no-op; picker is darwin-only (manual path fallback exists).

- Multi-window layout contention: two full pit-wall pages on the same origin both persist `garage-layout:<ws>` and can clobber each other's panel sets (observed once during e2e with a stale pre-restart page open; unreproducible single-window). Candidate fix: layout writes tagged with a window id + last-writer-wins guard, or refuse dockview persistence when another pit-wall window holds a lease. (2026-07-19)
