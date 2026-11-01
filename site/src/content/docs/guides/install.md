---
title: Install on macOS, Linux and Windows
description: Install claude-garage with npx, a shell installer or PowerShell. Needs tmux 3.2 or newer and Claude Code. Fix common errors and uninstall cleanly.
sidebar:
  order: 1
---

To install claude-garage, run `npx claude-garage`. You need tmux 3.2 or newer (macOS and Linux), Claude Code on your `PATH`, and Node.js 20 or newer for `npx` only. The shell and PowerShell installers need no Node.

Verified on claude-garage 1.0.0-rc.1: the commands, requirements and messages on this page were checked against that version's source on 2026-11-01. The Windows preview has not yet been run on real Windows.

## What do I need before I install claude-garage?

| Requirement | Needed for | Check it with |
|---|---|---|
| macOS or Linux (x64 or arm64). Windows x64 is a preview. | Everything | `uname -sm` |
| [Claude Code](https://docs.claude.com/en/docs/claude-code) on your `PATH` | Every session runs `claude` | `claude --version` |
| tmux 3.2 or newer | macOS and Linux. Windows does not use tmux. | `tmux -V` |
| Node.js 20 or newer | Only the `npx` method | `node --version` |

See [Platforms](/docs/platforms/) for how sessions run on each operating system.

## How do I install claude-garage?

1. Pick one install method. Each one downloads a prebuilt binary. Nothing is compiled.

   Run it with npx:

   ```bash
   npx claude-garage
   ```

   <!-- REMOVE AT 1.0.0: delete this sentence and the next code block when 1.0.0 is released and `npx claude-garage` serves it. -->
   While 1.0.0 is a release candidate, use the `next` tag:

   ```bash
   npx claude-garage@next
   ```

   Or use the shell installer (macOS and Linux):

   ```bash
   curl --proto '=https' --tlsv1.2 -LsSf \
     https://github.com/adar2378/claude-garage/releases/latest/download/claude-garage-installer.sh | sh
   ```

   It prints `claude-garage installed. Run claude-garage (needs tmux and the claude CLI on PATH).` when it is done. Read the [security note](#is-it-safe-to-pipe-the-installer-into-a-shell) first if you want to inspect the script.

   Or use the PowerShell installer (Windows preview):

   ```powershell
   powershell -ExecutionPolicy Bypass -c "irm https://github.com/adar2378/claude-garage/releases/latest/download/claude-garage-installer.ps1 | iex"
   ```

2. Make sure tmux is installed. On macOS, claude-garage offers to run `brew install tmux` for you. It asks first. On Linux, install it yourself:

   ```bash
   sudo apt install tmux
   ```

   Use `dnf install tmux` or `pacman -S tmux` on other distributions.

3. Start the wall:

   ```bash
   claude-garage
   ```

   The shell installer puts the binary in `~/.local/bin`. Add that folder to your `PATH` if the command is not found.

## What should I see after install?

The first launch checks for tmux and `claude`. If one is missing, claude-garage prints what to install and stops.

Then claude-garage starts its background daemon and opens the wall. With no projects yet, the wall shows a short message: press `w` to add one. Go on with [Your first wall](/docs/guides/first-wall/), or [try the wall in your browser](/try/) first.

## What does the daemon do?

The daemon is a small background program. It starts and watches your Claude sessions. claude-garage starts it for you, and it keeps running after you quit the wall.

- It listens on `127.0.0.1` only. Port `4747` is the default.
- Its log is `~/.garage/daemon.log`. Your state is in `~/.garage/state.json`.
- Press `q` in the wall to quit. The daemon and every session keep running. This works because [tmux owns your Claude Code sessions](/blog/why-tmux-owns-your-sessions/).
- To restart only the daemon, run `claude-garage restart`, or press `r d` in the wall. Your sessions are not touched.
- If port 4747 is taken, set `GARAGE_PORT` to another number. See [Configuration](/docs/guides/configuration/).

## How do I see what the launcher does?

A normal start prints nothing before the wall opens. To see what the launcher does, set `GARAGE_VERBOSE`:

```bash
GARAGE_VERBOSE=1 claude-garage
```

The launcher then prints lines such as "no daemon on port 4747, starting one". It also tells you when it replaces an old daemon. Errors and questions always print, with or without this variable.

## How do I upgrade from 0.6.x?

Run `claude-garage` again. It finds the old daemon, stops only that process and starts the new one. No session is closed. The [changelog](/docs/changelog/) lists what changed in each version.

## Troubleshooting

### `command not found: claude-garage`

Your shell cannot find the binary. The shell installer puts it in `~/.local/bin`. Check that folder is on your `PATH`:

```bash
echo "$PATH" | tr ':' '\n' | grep -x "$HOME/.local/bin"
```

If that prints nothing, add `export PATH="$HOME/.local/bin:$PATH"` to your shell profile and open a new terminal. With `npx`, no `PATH` change is needed. Run `npx claude-garage` instead.

### `tmux not found`

claude-garage needs tmux on macOS and Linux and stops when it is missing. On macOS with Homebrew, it asks `tmux not found. Install it now with Homebrew? [y/N]`. Answer `y` to run `brew install tmux`. Elsewhere it prints the commands: `apt install tmux`, `dnf install tmux` or `pacman -S tmux`. Then run `claude-garage` again.

### `claude not found`

The launcher could not run `claude --version`, so Claude Code is not installed or is not on the `PATH` of this terminal. The message points to the [Claude Code install instructions](https://docs.claude.com/en/docs/claude-code). Install it, open a new terminal, check `claude --version`, and run `claude-garage` again.

### My tmux is older than 3.2

claude-garage asks for tmux 3.2 or newer. It does not check the version at startup. It only checks that tmux exists. On older versions it skips two tmux control-mode options that need 3.2, so upgrade tmux instead of relying on it. Check with `tmux -V`, then upgrade with `brew upgrade tmux` or your package manager.

### `cannot bind 127.0.0.1:4747; is it taken?`

Another program already uses the daemon port. The full message ends with `set GARAGE_PORT to use another port`. Pick a free port:

```bash
GARAGE_PORT=4800 claude-garage
```

See [Configuration](/docs/guides/configuration/) for making it permanent. If an old claude-garage daemon holds the port, `claude-garage restart` replaces it without touching sessions.

### `claude-garage daemon did not come up`

The wall waited for the daemon and gave up. The full message names the log file. Read the end of `~/.garage/daemon.log`. A taken port is the most common cause, covered above.

## Is it safe to pipe the installer into a shell?

Piping a download into `sh` or `iex` runs the script before you have read it. The commands above fetch the installer from this project's GitHub Releases over HTTPS only (`--proto '=https' --tlsv1.2`), but you are still trusting that download. To read it first:

```bash
curl --proto '=https' --tlsv1.2 -LsSf -o claude-garage-installer.sh \
  https://github.com/adar2378/claude-garage/releases/latest/download/claude-garage-installer.sh
less claude-garage-installer.sh
sh claude-garage-installer.sh
```

On Windows, download `claude-garage-installer.ps1` from the [releases page](https://github.com/adar2378/claude-garage/releases), read it, then run it. If you prefer not to run an installer at all, use `npx claude-garage`. Report a security problem through the [GitHub security advisories](https://github.com/adar2378/claude-garage/security/advisories/new).

## How do I uninstall claude-garage?

These steps are for macOS and Linux. The Windows preview has no tested uninstall steps yet.

1. Quit the wall with `q`. Sessions and the daemon keep running.
2. Optional: end the sessions. They are tmux sessions named `garage/<project>/<label>`, so they would keep running otherwise. Kill them one at a time, or all at once:

   ```bash
   tmux list-sessions -F '#{session_name}' | grep '^garage/' | while read -r s; do tmux kill-session -t "=$s"; done
   ```

3. Stop the daemon. The launcher itself suggests this command when it cannot stop an old daemon:

   ```bash
   lsof -ti tcp:4747 | xargs kill
   ```

4. Remove the binary. If you used the shell installer:

   ```bash
   rm ~/.local/bin/claude-garage
   ```

   If you only used `npx`, nothing was installed on your `PATH`, so there is nothing to remove.

5. Remove garage's files. This deletes your saved session records, so closed chats can no longer be restored:

   ```bash
   rm -rf ~/.garage
   ```

   If you set `GARAGE_DIR`, remove that folder instead.

6. If you pressed `I` in the wall, claude-garage added hooks and a statusline to `~/.claude/settings.json`. Before each write it saved a copy named `settings.json.garage-backup-<timestamp>` in the same folder. Restore that copy, or remove the entries that point at files in `~/.garage` by hand. If you set `GARAGE_CLAUDE_HOME`, look there.

Your sessions' Claude conversations stay in Claude Code's own history. Removing claude-garage does not delete them.

## Next

Start with [Your first wall](/docs/guides/first-wall/). Then see [how to run many sessions](/docs/guides/many-sessions/), [keep sessions alive](/docs/guides/nothing-lost/), or the [key reference](/docs/reference/keys/). If you are weighing options, read [claude-garage vs tmux on its own](/compare/tmux-alone/).

If garage works for you, [star it on GitHub](https://github.com/adar2378/claude-garage). It is free and built by one person, and stars are how other Claude Code users find it.
