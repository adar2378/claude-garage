---
title: "Why tmux owns your sessions"
description: "In claude-garage, tmux runs every Claude Code session and the wall is only a viewer. Why that keeps sessions alive and lets restarts resume chats."
date: 2026-11-01
tags: [design, tmux, sessions]
---

In claude-garage, tmux runs your Claude Code sessions. The wall you look at is only a viewer. That choice is the reason sessions survive closing the terminal, and the reason a restart can resume a conversation instead of starting over.

## Who owns a session in claude-garage?

tmux does. Each Claude Code session is a tmux session named `garage/<project>/<label>`. The garage background service starts, lists and watches them, and the terminal wall attaches to them to draw tiles. The README shows the layers like this: tmux runs every session, the daemon sits on top of it, and the wall sits on top of the daemon.

The practical result is stated in the docs: you can delete garage and your sessions keep running. You can reattach to any one with:

```bash
tmux attach -t garage/<project>/<label>
```

## Why not let the wall run the sessions?

If the wall owned the sessions, they would end when the wall ended. That is the problem with terminal tabs: the tab is the session, so closing it, losing an SSH link or quitting the app takes the work with it.

By moving ownership to a process that is built to outlive its clients, garage turns every "close" into a harmless event:

- Press `q`, and the wall quits. The sessions keep running.
- Close the terminal window, and nothing is lost. Run `claude-garage` later and every session is there.
- Restart the garage service with `r d`, and sessions are not touched.

This is also why the wall is cheap. It does not have to keep anything alive, so it can be a small viewer.

## What happens after a reboot?

A reboot ends tmux and all of its sessions. tmux cannot save you there, so garage keeps its own record. It remembers each session id and folder, and it keeps the Claude conversation id to resume.

After a reboot you run `claude-garage`. Stopped sessions show the `⟳` glyph. Press `Enter` on one to restore it, or `R` to restore every stopped session in the project. garage starts Claude with `claude --resume`, and the tile shows the earlier conversation again.

If the project folder is gone, the session stays restorable and the strip tells you why. The record is kept, so you can try again after you fix the folder.

## How do restarts keep the conversation?

Claude Code updates do not reach a running session. A session keeps the version it started with. To move to the latest `claude`, press `r r` on a session, `r a` for all idle sessions in a project, or run `claude-garage restart --sessions`.

The restart is done in place with tmux's `respawn-pane`, so the tmux session keeps its identity, and Claude resumes the conversation. Busy sessions are skipped unless you pass `--all`, so a restart does not cut off work in progress.

## What does tmux ownership cost?

Three honest costs:

1. **tmux is a requirement.** On macOS and Linux you need tmux 3.2 or newer. garage offers to install it with `brew`.
2. **garage does not adopt sessions it did not start.** A session you began in a plain terminal is not managed. Adopting an existing tmux session is on the roadmap.
3. **Windows works differently.** tmux is not native there. The Windows preview uses one detached `claude-garage host` process per session. Those hosts outlive the service, the wall and the terminal, but not a reboot.

## What else does a long-lived session need?

Because sessions outlive the wall, garage adds a few safeguards for the moments in between. It reads the prompt box of each open tile once a second and keeps a copy of text you typed but did not send, so a crash does not lose it. After a restart, press `u` on the session to type the draft back. You can turn drafts off with `GARAGE_DRAFTS=0`.

Want the full flow? Read [Nothing gets lost](/docs/guides/nothing-lost/). Comparing approaches? See [claude-garage vs tmux on its own](/compare/tmux-alone/).
