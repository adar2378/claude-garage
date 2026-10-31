# @@TITLE@@

A [claude-garage](https://github.com/adar2378/claude-garage) plugin written in @@LANG@@.
It adds one action, **Say hello** (`@@ID@@:greet`), and one widget on the status strip.

## Try it

```sh
claude-garage plugin dev .       # links this folder, reloads on every change, tails the log
```

Then run the action from the palette, or bind a key:

```sh
claude-garage plugin bind @@ID@@:greet g
```

`plugin dev` asks you to approve the permissions in `garage-plugin.toml` the first time.
Stop it with Ctrl+C; the plugin stays linked (`claude-garage plugin remove @@ID@@` unlinks it).

## Test

```sh
@@TEST_CMD@@
```

## Publish

1. Push this folder to a public GitHub repository (the manifest must be at the repository root,
   or in a folder people install with `claude-garage plugin install owner/repo/folder`).
2. Add the GitHub topic `claude-garage-plugin` so `claude-garage plugin search` finds it.
3. People install it with `claude-garage plugin install owner/repo`. garage shows them every
   command and permission first. Plugins are not sandboxed: keep the permission list honest.

See `CLAUDE.md` for the manifest rules.
