---
title: Coming from terminal tabs
description: Move from many terminal tabs of Claude Code to one claude-garage wall. See what changes, what stays the same, and how to switch over without losing work.
sidebar:
  order: 7
---

You run Claude Code in several tabs or windows today. garage puts all of them on one screen and tells you which one waits for you.

## What changes

- **One window.** Every session is a tile in one terminal. You do not hunt through tabs.
- **tmux owns the sessions.** A tab used to be the session. Now tmux runs it, and a tile only shows it. Closing the terminal does not end anything.
- **Two modes.** In garage mode, single keys control the wall. To type to Claude, engage a tile with `Enter`. To leave, press `Ctrl+G`.
- **A signal for waiting.** A session that needs you turns amber. Press `a` to jump to it.
- **Sessions survive a reboot.** Restore them and Claude resumes the conversation.

## What stays the same

- It is the real Claude Code. Every key you press in an engaged tile goes to Claude, except `Ctrl+G`. `Esc`, `Shift+Tab` and arrow keys all work as before.
- Your Claude settings, hooks and projects are untouched. Press `I` to add status hooks and the context meter. It asks for nothing else.
- You can still open a session in its own window. Press `t` (macOS). Or attach with `tmux attach -t garage/<project>/<label>`.
- Everything stays on your machine. The daemon listens on `127.0.0.1` only. There is no account and no telemetry.

## Steps

1. Finish or park the work in your old tabs. garage does not adopt sessions you started outside it.
2. Run `claude-garage`.
3. Press `w` for each project folder you used to open in a tab. Or run `claude-garage ~/dev/app` after you enable the `cli-extras` plugin (`claude-garage plugin enable cli-extras`).
4. Press `n` in each project to start a session. One session per task works well.
5. To continue an old conversation, engage a tile and use Claude Code's own `/resume` command.
6. Close the old tabs.

## Tips

- Keep related repos in one parent folder. garage groups them in the rail under the folder name.
- Use `N` for tasks that touch the same files. Each gets its own git worktree. See [Worktrees](/guides/worktrees/).
- Learn four keys first: `n` new session, `Enter` engage, `Ctrl+G` leave, `a` jump to a waiting session.
- Press `m` to maximize a tile when you need the full screen. Press it again to go back.
- Use a terminal with good key support. Ghostty, iTerm2, kitty and WezTerm work best. In macOS Terminal.app, `Shift+Enter` acts like `Enter`. Use `Option+Enter` for a new line.
- A wall shows at most 6 tiles per view. Extra sessions stay in the rail. Press `d` to split a session into its own view.

## Next

[Configuration](/guides/configuration/)
