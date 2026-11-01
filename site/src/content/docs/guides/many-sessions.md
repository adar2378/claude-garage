---
title: Run many sessions at once
description: Run many Claude Code sessions at once in claude-garage. Group projects by folder, spot the amber session that waits for you, and jump to it with one key.
sidebar:
  order: 3
---

With many Claude Code sessions, the hard part is knowing which one waits for your answer. claude-garage marks that session amber and jumps to it when you press `a`. Sessions are grouped by project folder, and `A` lists every waiting session, longest wait first.

## How do I run several sessions at once?

1. Add each project with `w`. Switch between projects with `1` to `9`.
2. Start sessions with `n`. Use one tile per task.
3. Work in one tile. Press `Ctrl+G` when you want to leave it.
4. Watch the rail and the strip. When a session turns amber, press `a`.
5. Answer Claude. Press `Ctrl+G` when you are done.
6. Press `a` again for the next waiting session.

## How do I know which session needs me?

Amber means one thing: Claude waits for your answer. garage uses amber for nothing else.

- The tile border and title turn amber.
- The rail row turns amber and shows how long it has waited.
- The project tab in the strip gets an amber dot.
- garage rings the terminal bell once and puts the number of waiting sessions in the terminal title.
- A desktop notification appears when the garage terminal is hidden.

Waiting sessions are always sorted first in the rail.

<img src="/renders/wall-needs-input.svg" width="1440" height="880" loading="lazy" decoding="async" alt="A sample claude-garage wall: a rail of projects on the left and session tiles on the right, with one tile outlined in amber and a status strip saying a session needs you.">

Try it without installing: the [browser tutorial](/try/) simulates this exact wall, and the [key reference](/docs/reference/keys/) lists every key.

## How do I jump to the waiting session?

Press `a` to go to the session that has waited the longest. It works from any project. garage changes project and view if it must, and then it **engages** the tile. Your next keystroke goes straight to Claude.

For a permission prompt, this is two keys: `a`, then your answer. If no session waits, `a` shows a short notice and does nothing.

## How do I see every waiting session?

Press `A` to open the queue of all waiting sessions. The longest wait is at the top. Each row shows the session, the wait time, and Claude's message when there is one.

- `j` and `k` move the selection.
- `Enter` jumps to the selected session, the same way `a` does.
- `Esc` closes the queue.

You can also click the waiting count in the strip.

## How does claude-garage group projects?

Projects that live in the same parent folder sit together in the rail. A faint label with the folder name sits above the group. For example `et-backend` and `et-admin` in `elite-traders/` get an `elite-traders` label.

- If you also add the parent folder itself as a project, it leads the group.
- If any project in a group has a waiting session, the whole group moves up. The blocked project comes first.
- A project with no neighbor gets no label.
- Project numbers `1` to `9` do not change. Label rows have no number and ignore clicks.

## How do I see what finished?

When Claude finishes, the glyph turns green `✓`. After 2 minutes the green fades to grey. The glyph stays. The fade tells you what finished since you last looked, without asking for attention.

## How do I split one project into views?

Press `d` to move the focused session into its own view. Press `d` again to move it back. Press `D` to move it to another view. When a project has two or more views, a view bar shows above the grid. Press `Tab` to cycle views. A waiting session in any view still turns the project amber, and `a` finds it.

## Next

[Sessions in git worktrees](/docs/guides/worktrees/)

Related reading: [keep sessions alive after you close the terminal](/docs/guides/nothing-lost/), [why tmux owns your Claude Code sessions](/blog/why-tmux-owns-your-sessions/), and [claude-garage vs one session at a time](/compare/one-session-at-a-time/).
