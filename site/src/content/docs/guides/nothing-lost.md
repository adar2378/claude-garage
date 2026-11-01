---
title: "Claude Code sessions that survive a reboot"
description: tmux keeps your Claude sessions alive when you close the terminal. After a reboot, restore them with their full conversation. Recover unsent drafts with u.
sidebar:
  label: Nothing gets lost
  order: 5
---

garage does not run your sessions. tmux does. Close the wall, close the terminal, or lose your SSH link, and the sessions keep going.

## Close and come back

1. Press `q` to quit the wall. Or just close the terminal window.
2. Your sessions and the daemon keep running.
3. Run `claude-garage` later. Every session is there.

You can even delete garage and keep your sessions. Attach to one by hand:

```bash
tmux attach -t garage/<project>/<label>
```

## After a reboot

A reboot ends tmux and all its sessions. garage remembers each session id and folder.

1. Run `claude-garage`.
2. Sessions that stopped show the `⟳` glyph and a restore message in the tile.
3. Press `Enter` on one to bring it back. Press `R` to bring back all stopped sessions of the project.

garage starts Claude with `claude --resume`. The tile shows the earlier conversation again. If a project folder is gone, that session stays `⟳` and the strip tells you why. Its record is kept, so you can try again after you fix the folder.

To forget a stopped session, press `x x` on it. On a running session, `x x` kills it and keeps no history. Press `x X` instead to close it and keep it in the rail as a grey row.

## Update Claude Code without losing context

A running session keeps the Claude Code version it started with. To move sessions to the latest `claude`:

| Key | Action |
|---|---|
| `r r` | Restart the focused session. It resumes its conversation. |
| `r a` | Restart all idle and done sessions in this project. |
| `r d` | Restart garage's daemon. Sessions are not touched. |

`r r` on a busy session warns first. From a shell:

```bash
claude-garage restart              # the daemon only
claude-garage restart --sessions   # also idle and done sessions
claude-garage restart --all        # also busy sessions
```

Busy sessions are skipped unless you pass `--all`.

## Draft recovery

garage reads the prompt box of each open tile once a second. If you typed something and did not send it, garage keeps a copy. Images you pasted into the prompt are kept too. This protects you from a crash or a power cut.

1. After a restart, open the session that held the draft.
2. If its prompt is empty and garage has a draft, the strip says: "unsent draft kept for NAME - press u to restore it".
3. Focus that session and press `u`. garage types the text back, and then the images.

Good to know:

- The offer lasts 60 seconds. Only the focused session takes `u`.
- If you start typing while garage restores, the restore stops. It never overwrites your input.
- A draft is cleared once you send the prompt.
- Files live in `~/.garage/drafts`. They are private to your user. garage removes files after 7 days.

### Turn drafts off

Set the variable for one run:

```bash
GARAGE_DRAFTS=0 claude-garage
```

Or set it for good in `~/.garage/config.toml`:

```toml
drafts = false
```

With drafts off, garage keeps nothing and removes what an earlier run kept. See [Configuration](/docs/guides/configuration/).

## Next

[Plugins](/docs/guides/plugins/)
