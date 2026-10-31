---
title: Sessions in git worktrees
description: Give each Claude session its own git worktree and branch so parallel work never collides. Merge, discard or keep the branch when you close the session.
sidebar:
  order: 4
---

Two sessions in one checkout can overwrite each other's files. A worktree session works in its own folder on its own branch. You decide what happens to the branch when you are done.

## Start a worktree session

1. Add a project that is a git repository.
2. Focus the project and press `N` (capital n).
3. A new tile opens. Claude starts inside the new worktree, not in your main checkout.

garage runs `git worktree add` from the current `HEAD` of the repository. It creates:

- a folder at `~/.garage/worktrees/<project>/<label>`
- a branch named `garage/<label>`

If that name is taken, garage adds a number, for example `garage/feature-2`. If the project is not a git repository, garage refuses and creates nothing.

## Finish a worktree session

1. Press `x` twice (`x x`) on the worktree session. The first press arms the close. The strip says "press x again". Any other key cancels it. The second press must come within 3 seconds.
2. The session closes. A finish panel opens. It names the branch and the branch that is checked out in your repository (the merge target).
3. Choose one:

   | Key | Action |
   |---|---|
   | `m` | **Merge.** Merge the branch into the checked-out branch with `git merge --no-ff`. Then remove the worktree and delete the branch. |
   | `d` then `d` | **Discard.** Force-remove the worktree and delete the branch. The second `d` confirms. |
   | `k` or `Esc` | **Keep.** Close the panel and change nothing. |

The panel also opens when you close a session that is `⟳` restorable.

## What you'll see

- Merge succeeds: the panel closes and the strip names the branch and the result.
- The worktree has uncommitted changes: the panel stays open and shows the message. Merge is refused. Commit the changes in the session first, or discard.
- Merge conflict: garage runs `git merge --abort`, so your repository is never left in a half-merged state. The panel stays open with git's message. The worktree and branch stay as they were. Keep, then fix it by hand.

Discard only works on `garage/` branches. garage never deletes any other branch.

## Tips

- Keep a worktree session after a close by pressing `k`. The folder and branch stay. You can merge them later with normal git.
- A worktree session survives a reboot like any other session. Restore it and it opens in the same worktree folder. If you deleted the folder by hand, restore fails and says so.

## Next

[Nothing gets lost](/docs/guides/nothing-lost/)
