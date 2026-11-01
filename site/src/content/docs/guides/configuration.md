---
title: Configuration
description: "Every claude-garage setting that exists: environment variables, the config.toml file, and the switches for animations, drafts and tmux control mode."
sidebar:
  order: 8
---

garage works without any setup. This page lists the settings you can change.

## The garage folder

garage keeps its files in `~/.garage`. Set `GARAGE_DIR` to use another folder. Files in it:

- `config.toml`: your settings (you create it)
- `state.json`: sessions and projects
- `wall.json`: wall layout, views and the pet choice
- `daemon.log`: the daemon's log
- `drafts/`: kept drafts
- `worktrees/`: worktree sessions

## config.toml

Create `~/.garage/config.toml`. It has two options. Both are `true` or `false`.

```toml
animations = true
drafts = true
```

| Option | Default | Effect |
|---|---|---|
| `animations` | `true` | Fade-ins, the attention pulse and the closing dissolve. Set `false` for a still wall. |
| `drafts` | `true` | Keeps unsent prompts so you can restore them with `u`. Set `false` to keep nothing. |

The file is read when the wall starts. Restart the wall after a change.

## Environment variables

Set a variable in front of the command, or export it in your shell profile.

```bash
GARAGE_PORT=4800 claude-garage
```

| Variable | Default | Effect |
|---|---|---|
| `GARAGE_PORT` | `4747` | The daemon port. It always binds `127.0.0.1`. |
| `GARAGE_DIR` | `~/.garage` | The folder for garage's files. |
| `GARAGE_VERBOSE` | off | `1`, `true` or `yes` makes the launcher print what it does while it starts. |
| `GARAGE_NO_MOTION` | off | Turns animations off. `NO_MOTION` and `REDUCE_MOTION` work the same way. Empty, `0`, `false`, `no` and `off` count as not set. |
| `GARAGE_DRAFTS` | on | `0`, `false`, `no` or `off` turns draft recovery off. |
| `GARAGE_TMUX_CONTROL` | on | `0`, `false`, `no` or `off` turns off tmux control mode. See below. |
| `GARAGE_TMUX_SOCKET` | unset | Use a separate tmux server with this name. It also skips your `tmux.conf`. For testing and isolation. |
| `GARAGE_BACKEND` | `tmux` on macOS and Linux, `host` on Windows | `tmux` or `host`. The host backend runs each session in its own small process, with no tmux. |
| `GARAGE_CLAUDE_CMD` | `claude` | The program that sessions run. |
| `GARAGE_CLAUDE_HOME` | `~/.claude` | Where `I` writes the hooks and statusline. |
| `GARAGE_EDITOR_CMD` | `code` | The editor command the daemon uses to open a project folder. |

### Turn animations off

Use any one of these:

```bash
GARAGE_NO_MOTION=1 claude-garage
```

```toml
animations = false
```

### Turn drafts off

```bash
GARAGE_DRAFTS=0 claude-garage
```

```toml
drafts = false
```

When drafts are off, garage also removes the drafts it kept before. See [Nothing gets lost](/docs/guides/nothing-lost/).

### The tmux control off switch

By default the daemon opens a tmux control-mode client. It learns about session changes at once. It uses a hidden tmux session named `garage-control`. That session shows up in `tmux ls`. A bare `tmux attach` right after the daemon starts can land in it. Some tmux session savers can save it.

If this is a problem, turn control mode off:

```bash
GARAGE_TMUX_CONTROL=0 claude-garage
```

The daemon then checks sessions on a plain 2-second timer. Nothing else changes. The variable is read by the daemon, so restart the daemon after you set it: `claude-garage restart`. It must be set in the environment that starts the daemon.

## Plugin keys

Plugin settings are not in `config.toml`. Turn plugins on and off, and change their keys, with `claude-garage plugin enable`, `disable` and `bind`. See [Plugins](/docs/guides/plugins/).

## Next

Back to [Install](/docs/guides/install/), or open the Plugins docs section to write your own plugin.
