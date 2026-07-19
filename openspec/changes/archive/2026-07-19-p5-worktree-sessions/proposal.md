# Proposal: p5-worktree-sessions

## Why

Two sessions editing one working tree fight — mixed diffs, racing git state. Worktree-per-session is how claude-squad and Conductor isolate parallel agents, but they force it on every spawn, importing the merge-back problem into sessions that never edit anything. Garage's decided position (LATER.md, 2026-07-19): worktrees are **opt-in at spawn time** — isolation when you're parallelizing real work, zero friction when you're just asking questions.

## What Changes

- **Spawn opt-in**: `POST /api/sessions` accepts `worktree: true` — requires the workspace dir to be a git repo (400 otherwise); creates `git worktree add ~/.garage/worktrees/<workspace>/<label> -b garage/<label>` (branch collision → suffix) and starts claude *in the worktree*. The per-workspace `+` gains an "in worktree" toggle whose last state is remembered per workspace (localStorage).
- **Worktree metadata**: the session's resume metadata gains `{worktree: {path, branch, repoDir}}` so restore recreates the session in the worktree (not the registry dir) and the close flow knows what to offer.
- **Close flow for worktree sessions**: after the two-step ✕ kill, a small inline prompt offers **merge** (into the repo's current branch; conflict → error surfaced, worktree kept), **discard** (worktree + branch deleted), or **keep** (session gone, worktree left on disk for manual handling). Daemon: `POST /api/worktrees/finish {id, action}`.
- **Diff pane awareness**: when the focused session is a worktree session, the changes pane and review mode diff the *worktree*, not the workspace root (`GET /api/diff/:workspace?sessionId=` override). The pane header names the worktree branch.
- Branch chips (already shipped) make worktree sessions self-documenting: `⎇ garage/<label>` vs the root's branch — no extra UI needed for identification.

Out of scope: automatic PR creation, rebase flows, multi-repo worktrees, worktree support for restore of pre-p5 sessions.

## Capabilities

### New Capabilities
- `worktree-sessions`: spawn-time worktree creation and naming, metadata lifecycle, restore-into-worktree, and the merge/discard/keep finish flow.

### Modified Capabilities
- `session-lifecycle`: spawn contract gains the `worktree` flag and its validation.
- `workspace-diff`: the diff endpoint gains the focused-session worktree override.
- `pit-wall-ui`: new-session affordance gains the worktree toggle; close flow gains the finish prompt for worktree sessions.

## Impact

- Daemon: worktree module (create/finish, `git -C` only, never touches non-garage branches); sessions.js spawn/restore/delete paths; diff.js override param.
- UI: AddSessionControl toggle; close-flow prompt component; ChangesPane header/target; api wrappers.
- ~/.garage/worktrees/ becomes managed disk (removed on discard; orphan cleanup is manual for now — listed in LATER if it becomes a problem).
