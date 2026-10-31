---
title: "claude-garage vs one session at a time"
description: "Compare running Claude Code one session at a time with claude-garage: when single-session focus is enough and when parallel sessions need a wall."
short: "Running one Claude Code session at a time is simple. See when parallel sessions are worth a wall."
order: 3
otherLabel: "one session at a time"
intro: "One session at a time is the simplest way to use Claude Code: you give a task, wait, review, and then start the next. claude-garage is for the moment you want a second task running while the first one waits."
who:
  - "You often wait on Claude, and you have other tasks you could start in that time."
  - "You work on separate repos or separate branches of one repo at once."
  - "You want to answer a permission question quickly, wherever it appears."
rows:
  - { topic: "Focus", garage: "You can still maximize one tile with m and work as if it were alone.", other: "Full attention on one task." }
  - { topic: "Throughput", garage: "Several tasks run while you review another.", other: "One task at a time. Waiting time is idle time." }
  - { topic: "Review load", garage: "More sessions means more to review. You must keep up.", other: "Low. You review one result at a time." }
  - { topic: "File conflicts", garage: "N gives a session its own git worktree, so parallel edits do not collide.", other: "No conflict between sessions." }
  - { topic: "Permission prompts", garage: "A waiting session turns amber, and a jumps to it from any project.", other: "You are already looking at it." }
  - { topic: "Setup", garage: "Install a binary and tmux.", other: "None beyond Claude Code." }
  - { topic: "Cost", garage: "The tool is MIT licensed and free. Parallel sessions use more of your Claude usage.", other: "Lower usage per hour." }
notNeeded:
  - "Your tasks depend on each other, so you cannot start the next before the last is done."
  - "Your reviews are slow, so extra sessions would only queue up."
  - "You prefer a single quiet window."
faq:
  - q: "Will parallel sessions use more of my Claude usage?"
    a: "More sessions running at once spend your Claude plan or API usage faster. claude-garage does not change pricing. Its usage-bar plugin can show your 5-hour and weekly usage in the status strip so you can watch it."
  - q: "How do parallel sessions avoid editing the same files?"
    a: "Press N to start a session in its own git worktree. Each worktree has its own branch and working folder. When you close the session, you can merge the branch, discard it, or keep it."
  - q: "Can I use claude-garage with only one session?"
    a: "Yes. Press n for one session and m to maximize the tile. You still get restore after a reboot. If that is all you need, plain Claude Code is also fine."
  - q: "How many sessions are too many?"
    a: "It depends on how fast you review. A useful test is whether sessions wait on you for long. If several sit amber for minutes, run fewer."
---

## When parallel sessions pay off

The gain is in the waiting. Claude Code often stops to ask permission or finishes while you are busy. With one session, that time is lost. With a wall, you start another task and the amber tile tells you when to come back.

See [Many sessions](/docs/guides/many-sessions/) for how to organize them, and [Worktrees](/docs/guides/worktrees/) to keep parallel edits apart.
