---
title: "Know when Claude Code needs you"
description: "Get a chime and a desktop banner when a Claude Code session needs your answer or finishes a long turn. The amber tile, the a key, and the alerts plugin."
sidebar:
  label: Alerts
  order: 3
---

You do not have to watch the wall. claude-garage tells you when a Claude Code session needs you. It shows it on screen, and with the built-in `alerts` plugin it also makes a sound and a desktop banner.

## How does the wall show that a session needs me?

The tile turns amber. Amber means one thing: Claude waits for your answer. The rail row and the project tab turn amber too, so you can see it from any project.

- Press `a` to jump to the session that has waited the longest. Your next key goes straight to Claude.
- Press `A` to open the triage queue of every waiting session, longest wait first. `Enter` jumps to the selected one.

The [run many sessions](/docs/guides/many-sessions/) guide covers both keys in detail.

## What does the alerts plugin do?

`alerts` is one of the [built-in plugins](/docs/guides/plugins/). It is on by default. It makes:

- A chime of two rising notes when a session needs you.
- One soft note when a session finishes a turn that ran 20 seconds or more.
- A desktop banner when garage is not the focused window, such as "fix needs you" with Claude's own message.

Sessions that need you within two seconds of each other make one chime and one banner.

## How do I change the alerts?

Run `claude-garage alerts` to see the options.

```bash
claude-garage alerts                    # show the options
claude-garage alerts sound off          # no chime (on | off)
claude-garage alerts needs-input off    # no alert when a session needs you
claude-garage alerts done off           # no alert when a session finishes
claude-garage alerts banners off        # chime only, never a banner
claude-garage alerts test               # play both chimes and show a sample banner
claude-garage alerts terminal auto      # app a clicked banner brings forward (or a bundle id)
```

Use `claude-garage alerts test` after a change to hear and see the result.

## Can I click a banner to jump to the session?

On macOS, yes. Click a banner to jump to the session (needs `brew install terminal-notifier`). The click brings your terminal forward and the wall focuses that session. When several sessions need you, it focuses the one that has waited the longest. Without terminal-notifier you still get the banner, but a click does nothing.

claude-garage finds your terminal on its own (Ghostty, iTerm2, Terminal, WezTerm and kitty). For another one, give its bundle id: `claude-garage alerts terminal com.example.term`. Use `auto` to go back.

## How do I turn alerts off completely?

Press `p` in the wall to open the plugin list. Switch `alerts` off. claude-garage will not turn it back on.

## How do I make the status instant?

Press `I` in the wall to install the Claude Code hooks. This gives instant status, so a session shows as waiting as soon as Claude asks. The same step installs the context meter.

## Next

[Sessions in git worktrees](/docs/guides/worktrees/)
