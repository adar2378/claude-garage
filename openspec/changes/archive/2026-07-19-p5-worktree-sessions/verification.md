# p5 e2e verification — 2026-07-19

Scratch git repo registered as workspace `wt-demo`; verified against a packaged instance (bin/garage.js).

## Spawn + isolation

- `POST /api/sessions {worktree:true}` → worktree at `~/.garage/worktrees/wt-demo/feature` on branch `garage/feature`, claude's pane cwd inside it ✅ (daemon gate additionally covered: collision suffixing, non-git-repo 400, restore-into-worktree, restore 409 when the worktree path is missing)
- Rail rendered `○ feature ⎇ garage/feature` — worktree sessions self-document via the existing branch chips ✅

## Diff override

- With the worktree session focused, the changes pane showed the **worktree's** diff (`M app.py +4 −1`) and the `garage/feature` branch header — not the pristine repo root; without `sessionId` the root diff is unchanged ✅

## Close → finish flow

- **Bug found & fixed:** closing from the ✕ *inside* the dockview tab made dockview double-dispose the tab (`resource already disposed`) — the refetch→reconcile→removePanel chain ran while the click's stack was still unwinding, and the finish toast was lost. Fixed by deferring the refetch one macrotask and containing disposal errors in `reconcile`. Re-verified: toast appears (`worktree for feature2: merge · discard · keep`).
- **Product gap found & fixed:** the *common* failure is an agent that edited but never committed — merge no-ops and `worktree remove` refuses with raw git stderr. Added an up-front dirty check returning a human message: *"worktree has uncommitted changes — commit them in the session first, or discard"* — verified live ✅
- Merge (after committing in the worktree): merge commit landed on `main` (`Merge branch 'garage/feature2'`), content present, worktree removed, branch deleted ✅
- Discard on the dirty orphan: worktree force-removed, branch force-deleted, repo left clean with only `main` ✅
- Guard (daemon gate): `discard` with `branch:"main"` refused — non-garage branches untouchable ✅
- Toast keeps its choices on error (verified with the raw-stderr case before the guard landed) ✅

## Verdict

**p5 gate: PASS.** Opt-in worktree isolation with a complete lifecycle: spawn → visible via branch chip → worktree-scoped review → merge/discard/keep — plus two real bugs caught and fixed by the e2e itself.
