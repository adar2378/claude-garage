---
title: Use and install plugins
description: Meet the ten built-in claude-garage plugins, turn them on or off, install more from GitHub with a safe preview, search for plugins and write your own.
sidebar:
  order: 6
---

A plugin adds a key, a button, a status widget, a panel or a command to garage. Ten plugins are built into the binary. You can add more from GitHub or write your own.

## The ten built-in plugins

Six are on after your first run. Four are off.

| Plugin | Default | What it does | Key |
|---|---|---|---|
| `usage-bar` | on | Shows your 5-hour and weekly Claude usage in the strip. It keeps the last figures across restarts. | none |
| `closed-chats` | on | Keeps closed chats in the rail as grey rows. A panel lists the recent ones. | `H` |
| `quick-send` | on | Sends one line of text to a session, or to every session of the project. | `s`, `S` |
| `tile-buttons` | on | Adds `x` (close) and `del` (delete) buttons to every tile title bar. | click |
| `pit-pet` | on | A pet lives in the strip and reacts to your sessions. The pet is off until you press `P`. | `P` |
| `alerts` | on | Plays a chime, and shows a desktop banner when no garage window is in view, when a session needs you or finishes a long turn. Options: `claude-garage alerts`. | none |
| `turn-markers` | off | Draws a marker in the tile gutter where each Claude turn ended. | none |
| `jump-palette` | off | Opens a fuzzy search over sessions and projects. Choose one to focus it. | `/` |
| `prune-empty-projects` | off | Removes a project from the rail when you close its last chat. | none |
| `cli-extras` | off | Adds `claude-garage <folder>`, `claude-garage open`, `install` and `prune` to the command line. | none |

More detail:

- **alerts**: see [Know when Claude Code needs you](/docs/guides/alerts/) for the options. Press `p` and switch it off to silence it for good.
- **usage-bar** takes its figures from the Claude Code statusline. Press `I` once to install it.
- **closed-chats**: `H` opens "Recent chats".
- **quick-send** needs `Enter` twice in its popup, so nothing is sent by accident. `S` skips sessions that wait for input. `s` on a waiting session asks for an extra `y` first, because your line would answer its prompt.
- **tile-buttons**: the first click arms the button. The second click runs it. `x` closes the session, and it stays in the rail as a grey row. `del` kills the session and keeps no history. There is no way back.
- **pit-pet**: `P` cycles off, Arthur the cat, Papito the duck, Segan the pup, and off again.
- **cli-extras**: after you enable it, `claude-garage ~/dev/app` adds the folder as a project, starts a session if it has none, and opens the wall. `claude-garage prune` lists projects with no session and asks before it removes them. It accepts `--yes` and `--dry-run`.

Press `?` in the wall. Keys from plugins are listed in a "Plugins" section.

## Turn a plugin on or off

1. List what you have:

   ```bash
   claude-garage plugin list
   ```

2. Enable or disable one:

   ```bash
   claude-garage plugin enable jump-palette
   claude-garage plugin disable usage-bar
   ```

3. Change a key if you like:

   ```bash
   claude-garage plugin bind quick-send:send t
   claude-garage plugin bind quick-send:send --clear
   ```

If you disable a plugin, it stays disabled. A built-in plugin cannot be removed, only disabled. Later releases add new plugins in the off state.

## Install a plugin from GitHub

Plugins are programs. They run as you. They are not sandboxed. Read the preview before you say yes.

1. Install by `owner/repo`. Add `/subdir` if the plugin is in a folder of the repository:

   ```bash
   claude-garage plugin install owner/repo
   ```

2. Read the preview. Nothing from the plugin has run yet. It shows:
   - the name, version, source and license
   - the permissions, with dangerous ones marked `DANGEROUS`
   - every command the plugin will run, as an exact argument list
3. Answer the question. For a plain plugin it reads `Install <id>? [y/N]`. Only `y` or `yes` continues. For a plugin with a dangerous permission it reads `Install <id> including DANGEROUS permissions (...)?`.
4. Pin a version when you can:

   ```bash
   claude-garage plugin install owner/repo --ref v1.2.0
   ```

Two permissions are dangerous. `sessions.input` can type into your sessions. `core.actions` can close or delete sessions.

For scripts, `--yes` skips the question but not the preview. A dangerous plugin also needs `--allow-dangerous`.

Updates ask again only if commands, permissions or download addresses changed:

```bash
claude-garage plugin update --all
```

### The confirm rule

A plugin can ask to close or delete a session. It can never confirm. The wall arms the action, and you must press a key or click a second time. A plugin cannot send "confirmed".

### Stop or remove a plugin

```bash
claude-garage plugin disable <id>              # stops it at once
claude-garage plugin remove <id> --purge       # deletes it and its data
claude-garage plugin logs <id> -f              # follow its log
```

## Find plugins

```bash
claude-garage plugin search
claude-garage plugin search quick notes --sort updated
```

Search lists public GitHub repositories with the topic `claude-garage-plugin`. The results are not reviewed. A result is not a recommendation. Use `--sort stars` (default) or `--sort updated`, and `--limit` to change the count.

## Write your own

1. Create a working plugin. Choose `rust` (default), `ts` or `shell`:

   ```bash
   claude-garage plugin new my-plugin --lang shell
   cd my-plugin
   ```

2. Start dev mode. It links the folder, rebuilds when files change, reloads the plugin and shows its log:

   ```bash
   claude-garage plugin dev
   ```

3. Edit `garage-plugin.toml` and your code. Save. The plugin reloads.
4. To share it, push it to a public GitHub repository and add the topic `claude-garage-plugin`. Others install it with `claude-garage plugin install you/my-plugin`.

The manifest, events, widgets, panes and the HTTP API are in the Plugins docs section: [Plugin documentation](/docs/plugins/).

## Next

[Coming from terminal tabs](/docs/guides/from-terminal-tabs/)
