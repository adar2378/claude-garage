# CLI commands and the fallback

A plugin can add words to the `claude-garage` command.

```toml
[[cli]]
name = "usage"
description = "Print usage"
command = ["{plugin_dir}/bin/usage", "cli"]
needs_daemon = false
```

Now `claude-garage usage 5h` runs `bin/usage cli 5h`: the words after the name are appended to the command. It works with the daemon down, because the CLI reads the plugin list itself.

The name matches `^[a-z][a-z0-9-]{0,31}$`. It cannot be a core command (`start`, `stop`, `status`, `restart`, `plugin`, `tui`, `daemon`, `help`, `version`) or the name of another plugin's command.

## The run

- The environment is the usual [plugin environment](api.md#environment), The run context has `trigger.kind = "cli"` and `trigger.args.argv` with the words. `focus.dir` is the caller's working directory. There is no focused session.
- Standard input, output and error go to the terminal, so the program can print and ask questions.
- `GARAGE_PORT`, `GARAGE_DIR`, `GARAGE_TMUX_SOCKET`, `GARAGE_TMUX_CONTROL` and `GARAGE_BACKEND` are passed through, so a `claude-garage` child process finds the same daemon. Other variables are cleared.
- `needs_daemon = true` makes `claude-garage` start a current daemon first. Set it if your command calls the API.
- Exit code 0 is success.

## Handing over to the wall

A CLI command often ends with "now show me". The plugin cannot start the wall itself. It asks the launcher instead: it writes `{"open_wall": true}` to the file named by `GARAGE_HANDOVER` and exits 0. The launcher then opens the wall in its own environment. A run that exits non-zero never opens the wall.

In Rust: `garage_plugin_sdk::request_wall()`. It returns `Ok(false)` when there is no launcher (the command was run by hand), so you can tell the user what to type.

## The fallback

```toml
[[cli]]
name = "open"
command = ["{plugin_dir}/bin/open"]
fallback = true
needs_daemon = true
```

A command with `fallback = true` also receives any first word that is not a known command, with all the words (not only the ones after a name). Only one installed plugin may do this. A second one is refused at load, and the message names the first.

The built-in `cli-extras` plugin uses it so that `claude-garage ~/dev/app` registers the folder and opens the wall. Its rules are worth copying:

- Check that the word is what you want (for example an existing folder) before you do anything. If it is not, exit non-zero with a clear message, so a typo does not start a daemon. The launcher checks that the word is an existing folder before it starts one for a fallback command that needs the daemon.
- Resolve `~`, relative and absolute paths against `focus.dir`. The Rust SDK has `garage_plugin_sdk::folder::resolve_folder` for this.
