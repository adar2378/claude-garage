---
title: First run
description: Add a project, start a Claude session, and learn the five session states in the claude-garage wall.
sidebar:
  order: 2
---

Start the wall:

```bash
claude-garage
```

1. Press `w` and add a project folder. Type, paste or drag the path in, or press `Ctrl+O` to pick a folder.
2. Press `n` to start a Claude session in it.
3. Press `Enter` to type into the session. Press `Ctrl+G` to get back to garage.
4. Press `?` any time to see all keys.

Press `q` to quit. Your sessions keep running in the background, because tmux owns them.

## Session states

| Glyph | State | Meaning |
|---|---|---|
| `●` | needs input | Claude is waiting for your answer. Always sorted first. |
| `◐` | working | Claude is running. |
| `✓` | done | Finished since you last looked. |
| `○` | idle | Waiting for your next request. |
| `⟳` | restorable | The session stopped (after a reboot, say). Press `Enter` to bring it back. |

Press `a` to jump to the session that has waited longest, from any project. Press `A` for the full queue.

## Next

- [All keys](/docs/reference/keys/)
- [Commands](/docs/reference/commands/)
