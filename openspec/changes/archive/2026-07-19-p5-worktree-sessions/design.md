# Design: p5-worktree-sessions

## Context

Sessions currently all spawn in the workspace's registered directory. The branch-chip feature already resolves each session's branch from its pane's live cwd, so worktree sessions become visually distinct with zero additional UI. The resume-metadata map in `~/.garage/state.json` already persists per-session records the poller maintains — the natural home for worktree bookkeeping.

## Goals / Non-Goals

**Goals:** opt-in isolation per spawn; worktrees that survive reboots (restore lands back in the worktree); a close flow that resolves the branch (merge/discard/keep) instead of leaking it; diff review scoped to the worktree when a worktree session is focused.

**Non-Goals:** forcing worktrees; PR automation; rebase/conflict resolution UI (merge conflicts surface as an error and keep the worktree — the user resolves in a terminal or editor); cleaning orphaned worktrees automatically; Windows.

## Decisions

**D-wt-location — `~/.garage/worktrees/<workspace>/<label>`, outside the repo.**
Inside-repo locations (`<repo>/.garage/worktrees`) pollute `git status` for every non-garage tool unless ignored, and `.gitignore` edits to user repos are off-limits. A central home under `~/.garage` keeps repos pristine, survives repo re-clones harmlessly, and makes "what has garage created?" one `ls`. Collision with an existing dir → suffix `-2`, `-3` (matches the picker's naming discipline).

**D-wt-branch — `garage/<label>`, suffixed on collision.**
Namespaced so a repo's real branches never collide; `git branch -d` on discard only ever touches `garage/*` branches created by us (guard: refuse to delete a branch not matching the recorded name). Branch is created at the repo's current HEAD.

**D-wt-meta — worktree record rides the existing session metadata.**
`state.json` sessions entry gains `worktree: {path, branch, repoDir}` written at spawn (not by the poller — spawn owns it, the poller never removes it). Restore uses `meta.worktree?.path ?? registered.dir` as the spawn cwd, with a stat-check → 409 "worktree missing" (metadata kept) if the path is gone. DELETE keeps its existing semantics (kill + remove meta) but returns the worktree record in the response body so the UI can offer the finish flow AFTER the kill: `{deleted: true, worktree: {...}}`.

**D-wt-finish — `POST /api/worktrees/finish {id?, worktree: {path, branch, repoDir}, action}` runs after the session is dead.**
Actions: `merge` → `git -C repoDir merge --no-ff garage/<label>` (non-zero exit → 409 with git's stderr, worktree and branch kept — the user resolves manually); on success → `git worktree remove <path>` + `git branch -d <branch>`. `discard` → `git worktree remove --force <path>` + `git branch -D <branch>` (guarded to `garage/*` names). `keep` → no-op (endpoint exists so the UI flow is uniform; simply dismissing the prompt is equivalent). The endpoint takes the worktree record from the DELETE response rather than re-deriving it, because the session (and its metadata) are already gone by then.

**D-wt-diff — `GET /api/diff/:workspace?sessionId=<id>` overrides the diff root.**
If the named session is live and its pane cwd (or its worktree meta) sits outside the registry dir, diff that path instead. The pane header shows `⎇ garage/<label>` when overridden. Review mode inherits automatically (same data source). Alternative (a separate `/api/diff/session/:id`) rejected — the workspace route already carries the UI's context and the override is one query param.

**D-wt-ui — toggle on the `+` control; finish prompt replaces the ✕ "sure?" for worktree sessions.**
`AddSessionControl` gains a `[wt]` checkbox (defaults from `localStorage garage-wt-default:<workspace>`, written on every spawn). For worktree sessions the ✕ two-step confirm becomes three-way on the second step: a compact inline row `merge · discard · keep` (each one click, no modal — consistent with the TUI aesthetic). Non-worktree sessions keep the existing behavior exactly.

## Risks / Trade-offs

- [Merge conflicts mid-finish] → 409 + stderr surfaced inline; worktree/branch kept; user resolves manually. No conflict UI in v1.
- [Orphaned worktrees (user chose keep, or crash between kill and finish)] → visible via `git worktree list` and `~/.garage/worktrees`; manual cleanup documented; auto-GC deferred to LATER if it bites.
- [Repo's current branch moves between spawn and merge] → merge targets whatever HEAD is at finish time — matches git norms; documented.
- [`git worktree add` fails on dirty/locked repo states] → error surfaced at spawn (400 + stderr), no tmux session created, nothing to clean.
- [Restore of a worktree session whose worktree was manually deleted] → 409 with reason, metadata kept, user can discard via finish with the recorded paths.

## Open Questions

- None blocking; auto-GC of kept worktrees and a conflict-resolution flow are explicitly deferred.
