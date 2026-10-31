---
title: Run many sessions at once
description: Work across several projects and sessions. Groups by folder, the amber needs-input signal, the a jump, the A queue and the done fade keep you in control.
sidebar:
  order: 3
---

The hard part of many sessions is knowing which one waits for you. garage shows it, and takes you there with one key.

## Steps

1. Add each project with `w`. Switch between projects with `1` to `9`.
2. Start sessions with `n`. Use one tile per task.
3. Work in one tile. Press `Ctrl+G` when you want to leave it.
4. Watch the rail and the strip. When a session turns amber, press `a`.
5. Answer Claude. Press `Ctrl+G` when you are done.
6. Press `a` again for the next waiting session.

## The amber signal

Amber means one thing: Claude waits for your answer. garage uses amber for nothing else.

- The tile border and title turn amber.
- The rail row turns amber and shows how long it has waited.
- The project tab in the strip gets an amber dot.
- garage rings the terminal bell once and puts the number of waiting sessions in the terminal title.
- A desktop notification appears when the garage terminal is hidden.

Waiting sessions are always sorted first in the rail.

## Jump with `a`

Press `a` to go to the session that has waited the longest. It works from any project. garage changes project and view if it must, and then it **engages** the tile. Your next keystroke goes straight to Claude.

For a permission prompt, this is two keys: `a`, then your answer. If no session waits, `a` shows a short notice and does nothing.

## The queue with `A`

Press `A` to open the queue of all waiting sessions. The longest wait is at the top. Each row shows the session, the wait time, and Claude's message when there is one.

- `j` and `k` move the selection.
- `Enter` jumps to the selected session, the same way `a` does.
- `Esc` closes the queue.

You can also click the waiting count in the strip.

## Groups by folder

Projects that live in the same parent folder sit together in the rail. A faint label with the folder name sits above the group. For example `et-backend` and `et-admin` in `elite-traders/` get an `elite-traders` label.

- If you also add the parent folder itself as a project, it leads the group.
- If any project in a group has a waiting session, the whole group moves up. The blocked project comes first.
- A project with no neighbor gets no label.
- Project numbers `1` to `9` do not change. Label rows have no number and ignore clicks.

## The done fade

When Claude finishes, the glyph turns green `✓`. After 2 minutes the green fades to grey. The glyph stays. The fade tells you what finished since you last looked, without asking for attention.

## Views

Press `d` to move the focused session into its own view. Press `d` again to move it back. Press `D` to move it to another view. When a project has two or more views, a view bar shows above the grid. Press `Tab` to cycle views. A waiting session in any view still turns the project amber, and `a` finds it.

## Next

[Sessions in git worktrees](/guides/worktrees/)
