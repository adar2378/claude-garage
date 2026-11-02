---
title: "Why tmux owns your Claude Code sessions"
description: "In claude-garage, tmux runs every Claude Code session and the wall is only a viewer. Why that keeps sessions alive and lets restarts resume chats."
date: 2026-11-01
tags: [design, tmux, sessions]
---

In claude-garage, tmux runs your Claude Code sessions. The wall you look at is only a viewer. That choice is the reason sessions survive closing the terminal, and the reason a restart can resume a conversation instead of starting over.

## Who owns a session in claude-garage?

tmux does. Each Claude Code session is a tmux session named `garage/<project>/<label>`. The garage background service starts, lists and watches them, and the terminal wall attaches to them to draw tiles. The [README](https://github.com/adar2378/claude-garage#how-it-works) shows the layers like this: tmux runs every session, the daemon sits on top of it, and the wall sits on top of the daemon.

<figure class="diagram">
<svg viewBox="0 0 560 330" width="560" height="330" role="img" aria-labelledby="tmux-diagram-title tmux-diagram-desc" xmlns="http://www.w3.org/2000/svg" font-family="ui-sans-serif, system-ui, sans-serif" font-size="15">
<title id="tmux-diagram-title">How claude-garage layers on tmux</title>
<desc id="tmux-diagram-desc">Four stacked boxes. At the top, the terminal wall, which only views sessions. Below it, the daemon, which starts, lists and watches sessions. Below that, tmux, which runs every session. At the bottom, the Claude Code sessions themselves, each a tmux session named garage, project, label. Quitting the wall or closing the terminal affects only the top box.</desc>
<rect x="20" y="10" width="520" height="62" rx="12" fill="none" stroke="currentColor" stroke-width="2" stroke-dasharray="6 4"/>
<text x="40" y="38" font-weight="650" fill="currentColor">Wall (the TUI you look at)</text>
<text x="40" y="58" fill="currentColor" opacity=".75">A viewer. Quit it and nothing else stops.</text>
<rect x="20" y="92" width="520" height="62" rx="12" fill="none" stroke="currentColor" stroke-width="2"/>
<text x="40" y="120" font-weight="650" fill="currentColor">Daemon (background service)</text>
<text x="40" y="140" fill="currentColor" opacity=".75">Starts, lists and watches sessions. Listens on 127.0.0.1.</text>
<rect x="20" y="174" width="520" height="62" rx="12" fill="none" stroke="currentColor" stroke-width="2"/>
<text x="40" y="202" font-weight="650" fill="currentColor">tmux</text>
<text x="40" y="222" fill="currentColor" opacity=".75">Runs every session. Outlives the wall and the daemon.</text>
<rect x="20" y="256" width="520" height="62" rx="12" fill="#E85002" fill-opacity=".12" stroke="#B33E00" stroke-width="2"/>
<text x="40" y="284" font-weight="650" fill="currentColor">Claude Code sessions</text>
<text x="40" y="304" fill="currentColor" opacity=".75">One tmux session each, named garage/&lt;project&gt;/&lt;label&gt;.</text>
<path d="M280 72v20M280 154v20M280 236v20" stroke="currentColor" stroke-width="2" fill="none"/>
</svg>
<figcaption>The wall sits on the daemon, the daemon sits on tmux, and tmux runs the sessions. Based on the layer diagram in the <a href="https://github.com/adar2378/claude-garage#how-it-works">README</a>.</figcaption>
</figure>


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

After a reboot you run `claude-garage`. Stopped sessions show the `⟳` glyph. Press `Enter` on one to restore it, or `R` to restore every stopped session in the project. garage starts Claude with [`claude --resume`](https://code.claude.com/docs/en/cli-reference), and the tile shows the earlier conversation again.

If the project folder is gone, the session stays restorable and the strip tells you why. The record is kept, so you can try again after you fix the folder.

## How do restarts keep the conversation?

Claude Code updates do not reach a running session. A session keeps the version it started with. To move to the latest `claude`, press `r r` on a session, `r a` for all idle sessions in a project, or run `claude-garage restart --sessions`.

The restart is done in place with tmux's [`respawn-pane`](https://man.openbsd.org/tmux.1#respawn-pane) (the code is in [`backend/tmux.rs`](https://github.com/adar2378/claude-garage/blob/main/crates/daemon/src/backend/tmux.rs)), so the tmux session keeps its identity, and Claude resumes the conversation. Busy sessions are skipped unless you pass `--all`, so a restart does not cut off work in progress.

## What does tmux ownership cost?

Three honest costs:

1. **tmux is a requirement.** On macOS and Linux you need tmux 3.2 or newer. garage offers to install it with `brew`.
2. **garage does not adopt sessions it did not start.** A session you began in a plain terminal is not managed. Adopting an existing tmux session is on the roadmap.
3. **Windows works differently.** tmux is not native there. The Windows preview uses one detached `claude-garage host` process per session. Those hosts outlive the service, the wall and the terminal, but not a reboot.

## What else does a long-lived session need?

Because sessions outlive the wall, garage adds a few safeguards for the moments in between. It reads the prompt box of each open tile once a second and keeps a copy of text you typed but did not send, so a crash does not lose it. After a restart, press `u` on the session to type the draft back. You can turn drafts off with `GARAGE_DRAFTS=0`.

## Why did I build it this way?

I wanted a simple TUI to manage my Claude Code sessions across different projects. The similar apps I tried before were quite complex and did too much. Letting tmux own the sessions is part of keeping claude-garage simple: the wall only has to be a viewer, so it stays small, and tmux does the one job of keeping sessions alive.

## Sources

- tmux manual: [`respawn-pane`](https://man.openbsd.org/tmux.1#respawn-pane).
- Claude Code docs: [CLI reference](https://code.claude.com/docs/en/cli-reference) (`--resume`).
- claude-garage source: [`crates/daemon/src/backend/tmux.rs`](https://github.com/adar2378/claude-garage/blob/main/crates/daemon/src/backend/tmux.rs) for how the daemon drives tmux, and [`README.md`](https://github.com/adar2378/claude-garage/blob/main/README.md) for the layers and privacy notes. Facts checked against version 1.0.1.

## Keep reading

- [Nothing gets lost](/docs/guides/nothing-lost/): the full close, reboot and restart flow.
- [claude-garage vs tmux on its own](/compare/tmux-alone/): what the wall adds on top of tmux.
- [How the plugin confirm rule keeps you in control of Claude Code](/blog/plugin-confirm-rule/).
- Ready to try it? [Install claude-garage](/docs/guides/install/), or [try the wall in your browser](/try/) first.
