---
title: Install claude-garage
description: Install claude-garage with npx, the shell installer or PowerShell. Check tmux and Claude Code, start the wall, and learn what the background daemon does.
sidebar:
  order: 1
---

claude-garage is one native binary. You install it once, then run `claude-garage` to open the wall.

## What you need

- macOS or Linux. Windows x64 works as a preview.
- [Claude Code](https://docs.claude.com/en/docs/claude-code) on your `PATH`. Check it with `claude --version`.
- tmux 3.2 or newer on macOS and Linux. Check it with `tmux -V`. Windows does not need tmux.
- Node.js 20 or newer, only if you install with `npx`.

## Steps

1. Pick one install method. Each one downloads a prebuilt binary. Nothing is compiled.

   Run it with npx:

   ```bash
   npx claude-garage
   ```

   While 1.0.0 is a release candidate, use the `next` tag:

   ```bash
   npx claude-garage@next
   ```

   Or use the shell installer (macOS and Linux):

   ```bash
   curl --proto '=https' --tlsv1.2 -LsSf \
     https://github.com/adar2378/claude-garage/releases/latest/download/claude-garage-installer.sh | sh
   ```

   Or use the PowerShell installer (Windows preview):

   ```powershell
   powershell -ExecutionPolicy Bypass -c "irm https://github.com/adar2378/claude-garage/releases/latest/download/claude-garage-installer.ps1 | iex"
   ```

2. Make sure tmux is installed. On macOS, garage offers to run `brew install tmux` for you. It asks first. On Linux, install it yourself:

   ```bash
   sudo apt install tmux
   ```

   Use `dnf install tmux` or `pacman -S tmux` on other distributions.

3. Start the wall:

   ```bash
   claude-garage
   ```

   The shell installer puts the binary in `~/.local/bin`. Add that folder to your `PATH` if the command is not found.

## What you'll see

The first launch checks for tmux and `claude`. If one is missing, garage prints what to install and stops.

Then garage starts its background daemon and opens the wall. With no projects yet, the wall shows a short message: press `w` to add one. Go on with [Your first wall](/guides/first-wall/).

## The daemon

The daemon is a small background program. It starts and watches your Claude sessions. garage starts it for you, and it keeps running after you quit the wall.

- It listens on `127.0.0.1` only. Port `4747` is the default.
- Its log is `~/.garage/daemon.log`. Your state is in `~/.garage/state.json`.
- Press `q` in the wall to quit. The daemon and every session keep running.
- To restart only the daemon, run `claude-garage restart`, or press `r d` in the wall. Your sessions are not touched.
- If port 4747 is taken, set `GARAGE_PORT` to another number. See [Configuration](/guides/configuration/).

## Quiet startup

A normal start prints nothing before the wall opens. To see what the launcher does, set `GARAGE_VERBOSE`:

```bash
GARAGE_VERBOSE=1 claude-garage
```

The launcher then prints lines such as "no daemon on port 4747, starting one". It also tells you when it replaces an old daemon. Errors and questions always print, with or without this variable.

## Upgrade from 0.6.x

Run `claude-garage` again. It finds the old daemon, stops only that process and starts the new one. No session is closed.

## Next

[Your first wall](/guides/first-wall/)
