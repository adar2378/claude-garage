# Tasks: p5-worktree-sessions

Groups 1 (daemon) and 2 (UI) parallelizable; group 3 integrates + verifies e2e.

## 1. Daemon

- [x] 1.1 worktrees.js module: create (add + branch, collision suffixing, 400 non-git repo, stderr surfaced), finish (merge --no-ff / discard with garage/* guard / keep), all `git -C`, never touching non-garage branches
- [x] 1.2 sessions.js: spawn accepts `worktree:true` → create worktree, spawn claude there, persist `worktree` record in session meta; restore prefers `meta.worktree.path` (409 + keep meta when missing); DELETE returns the worktree record
- [x] 1.3 diff.js: `?sessionId=` override → diff the session's worktree/live cwd when outside the registry dir
- [x] 1.4 Gate: curl e2e in a scratch git repo — spawn with worktree → `git worktree list` shows it, claude runs there, branch garage/<label>; diff override returns worktree changes; kill → DELETE body carries record; finish merge lands the commit in the repo; finish discard removes worktree+branch; non-garage branch name refused

## 2. UI

- [x] 2.1 AddSessionControl: `[wt]` toggle, default remembered per workspace (localStorage), passes worktree flag to spawn
- [x] 2.2 Close flow: worktree sessions' ✕ second step becomes inline `merge · discard · keep`; errors (conflict stderr) inline; non-worktree sessions unchanged
- [x] 2.3 ChangesPane: pass focused sessionId; header shows the worktree branch when the diff is overridden
- [x] 2.4 Gate: `npm run build -w ui` clean

## 3. Integration + e2e (the p5 gate)

- [x] 3.1 Full loop in the browser against a real scratch repo: spawn worktree session → ⎇ garage/<label> chip renders → make an edit via the session's terminal (or directly) → changes pane shows the worktree diff → close → merge → commit visible in repo log → worktree and branch gone
- [x] 3.2 Discard path: second worktree session, edit, close → discard → repo clean, worktree gone, branch gone
- [x] 3.3 Keep path + restore: worktree session survives daemon restart and restores INTO the worktree; keep leaves the worktree on disk
- [x] 3.4 Record in `verification.md`
