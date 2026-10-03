## Context

The Rust TUI is the product; the React web wall is legacy. Two features exist only in the browser:
- **Hooks install**: `POST /api/hooks/install` (idempotent, backup + atomic write). Only the web banner calls it. No endpoint reports whether hooks are installed.
- **Worktree finish**: `DELETE /api/sessions/*` returns `{deleted, worktree: {path, branch, repoDir} | null}` (also via `?meta=1` for restorable sessions). The web toast then calls `POST /api/worktrees/finish`. The TUI only prints "merge or discard in the web wall" (`wall/src/runtime.rs:1311`).

The TUI talks to the daemon over HTTP + SSE and attaches to tmux through its own PTY (`wall/src/pty.rs`). It never uses the `/term` WebSocket bridge, static serving, or `node-pty`.

## Goals / Non-Goals

**Goals:**
- TUI covers hooks install and worktree finish, so nothing points at the web wall.
- Delete the web wall and every dependency only it needs.
- Repo is shippable after the port and before the delete.

**Non-Goals:**
- Diff review in the TUI (roadmap). `diff.js` and `editor.js` stay in the daemon for it.
- A "hooks installed?" detector or new endpoint.
- Clicking a notification focusing the user's terminal.

## Decisions

**1. Fold hooks into `I` instead of a new key.** One key for "connect garage to Claude Code" is easier to learn. The existing hint heuristic (no statusline data after 30s) stays; the text changes. Alternative: a separate key with its own detector. Rejected: no endpoint exists and the web heuristic was a guess too.
- Effect runs statusline install, then hooks install, in one blocking task; both results go into one `AppEvent`, so one notice and one busy guard (`installing_statusline` renamed to `installing`).

**2. Finish overlay is a new `OverlayKind::WorktreeFinish`.** It follows the existing overlay pattern (`OverlayKind`, `open_overlay`, `handle_overlay_key`, render in its own `ui/worktree_finish.rs`, like `ui/workspace_add.rs`).
- The overlay owns the worktree record. Session state is gone after DELETE, so nothing may be re-derived.
- Keys: `m` merge, `d d` discard (reuse `ArmedAction`), `k`/`Esc` keep. Busy guard while a request is in flight; `AppEvent::WorktreeFinishSettled(Result)` closes the overlay on success or shows the error inline.
- The overlay is modal: it blocks other wall keys until resolved. The record would otherwise be lost.

**3. Merge target comes from the daemon.** The DELETE response's worktree record gains `target` (current branch of `repoDir`). The TUI only displays it. Alternative: TUI shells out to git. Rejected: keeps git in one place.

**4. `git merge --abort` on merge failure.** Today a conflict leaves `repoDir` mid-merge, which the spec forbids. The abort runs before the 409 response. Abort errors are ignored (nothing to abort).

**5. Bare `npx claude-garage` runs the TUI.** `tui` stays an alias. The web path in `bin/garage.js` (`main()`, `GARAGE_SERVE_UI`, browser `open`) is deleted. `startDetachedDaemon` stops setting `GARAGE_SERVE_UI`.

**6. Keep daemon routes the roadmap needs.** Keep `diff.js`, `editor.js`, the snippet routes and workspace rename. Delete `term.js`, static serving, Vite dev origins. Check `grep -rn "node-pty\|from \"ws\"" daemon/` before removing deps.

## Risks / Trade-offs

- [Users of the browser wall lose it, including diff review] → 0.5.0 with a clear CHANGELOG entry; diff review stays on the roadmap.
- [Overlay lost if the TUI quits while it is open] → Worktree and branch are kept (same as keep); the strip already said "worktree kept". Acceptable.
- [Security allowlist change breaks TUI requests] → TUI sends no Origin header; verify `security.js` still lets Origin-less requests through after removing the `:5173` and own-origin entries.
- [Verification touches real git and tmux] → Unit tests use scratch repos in the scratchpad. Live `x x` and merge/discard run in the main session only, with the user's consent.

## Migration Plan

1. Phase A (port): daemon `target` + abort, TUI `I` and overlay. Ship as 0.4.2 if desired.
2. Phase B (delete): remove web code and deps, bare command runs TUI. Ship as 0.5.0.
3. Rollback: reinstall `claude-garage@0.4.x`. State file (`~/.garage/state.json`) is unchanged by this change.

## Open Questions

- None blocking. Whether to ship Phase A separately as 0.4.2 is the user's call at release time.
