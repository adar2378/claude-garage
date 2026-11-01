---
title: "claude-garage vs tmux on its own"
description: "Compare claude-garage with plain tmux for Claude Code: what tmux gives you already, what claude-garage adds on top, and when plain tmux is better."
short: "claude-garage is built on tmux. See what it adds and what plain tmux already covers."
order: 2
otherLabel: "tmux on its own"
intro: "claude-garage does not replace tmux. On macOS and Linux, tmux runs every claude-garage session. The question is whether you want to build the rest yourself: status for each session, a jump key, restore after a reboot, and worktree handling."
who:
  - "You already like tmux but do not want to write scripts that find the Claude session waiting for you."
  - "You want sessions grouped by project with one key to jump to a blocked one."
  - "You want a restored session to resume its Claude conversation, not just open an empty shell."
rows:
  - { topic: "Keeping sessions alive", garage: "Yes. tmux runs every session.", other: "Yes. This is what tmux does best." }
  - { topic: "Layout", garage: "A wall of tiles per project, plus a rail of projects and sessions.", other: "You design windows and panes." }
  - { topic: "Which session waits for me", garage: "Amber marks it. a jumps to it, A lists every waiting session.", other: "Nothing built in. You write a script or watch the panes." }
  - { topic: "After a reboot", garage: "Records are kept. Enter restores a session with claude --resume. R restores all stopped sessions of a project.", other: "tmux sessions are gone. You restart and resume each by hand, or use a plugin." }
  - { topic: "Git worktrees", garage: "N creates one per session. Closing offers merge, discard or keep, and aborts a failed merge.", other: "Manual git commands." }
  - { topic: "Extending it", garage: "Plugins: widgets, keys, panes and CLI commands, written in any language.", other: "tmux config, scripts and tmux plugins." }
  - { topic: "Your own tmux setup", garage: "garage sessions are ordinary tmux sessions named garage/<project>/<label> on your tmux server. garage turns the tmux status line off for them, because the wall draws its own title bars.", other: "Full control: your config, your keys, your layout." }
  - { topic: "Windows", garage: "Preview. Uses a session host process instead of tmux.", other: "tmux is not native on Windows." }
notNeeded:
  - "You are happy to script your own status checks and restore steps."
  - "You want total control of tmux layout, keys and config."
  - "You work on remote servers where installing another tool is not an option."
faq:
  - q: "Do I need to know tmux to use claude-garage?"
    a: "No. You use the wall's keys. You only need tmux installed, version 3.2 or newer. If you know tmux, you can still attach to any session by hand with tmux attach -t garage/<project>/<label>."
  - q: "If I delete claude-garage, do I lose my sessions?"
    a: "No. The sessions are tmux sessions. You can remove claude-garage and they keep running, and you can attach to each one with tmux."
  - q: "Does claude-garage change my tmux config?"
    a: "It does not edit your tmux config. By default it uses your tmux server, so your tmux.conf still applies. It turns the tmux status line off for the sessions it creates, because the wall draws its own title bars."
  - q: "Can plugins do what my tmux scripts do?"
    a: "Often yes. A plugin can react to events such as a session changing status, push a status widget, or add a key. Plugins are programs that run as you and are not sandboxed, so read the install preview."
  - q: "What about Windows?"
    a: "Windows x64 is a preview. It has a prebuilt binary and uses one detached host process per session instead of tmux. It has not yet been verified on real Windows."
---

## What claude-garage adds on top of tmux

tmux gives you durable sessions. claude-garage adds Claude-specific knowledge: it knows which sessions are Claude Code, whether each one is working, idle, done or waiting for your answer, and which conversation to resume. The wall is a terminal app that shows those sessions as tiles by attaching to tmux.

If you only want durability, plain tmux is enough and costs nothing. Read [why tmux owns your sessions](/blog/why-tmux-owns-your-sessions/) for the design reasoning, and [Nothing gets lost](/docs/guides/nothing-lost/) for the restore flow.
