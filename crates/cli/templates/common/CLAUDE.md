# Writing this claude-garage plugin

You are editing a claude-garage plugin: a folder with `garage-plugin.toml` and a program that garage
runs. The plugin never runs inside garage. garage starts its commands, passes the run context, and the
plugin talks back over a local HTTP API. @@LANG_NOTES@@

## The dev loop

```sh
claude-garage plugin dev .        # link, rebuild and reload on change, tail the log
claude-garage plugin logs @@ID@@ -f  # the log alone
claude-garage plugin list         # state, problems, permissions
@@TEST_CMD@@
```

After you change `garage-plugin.toml`, `plugin dev` revalidates it and prints every problem with the
field path. Adding a permission needs the user to approve it again (it asks). Never tell the user to edit
`~/.garage/plugins.json`.

## Manifest rules (`garage-plugin.toml`)

- Required header: `id` (`^[a-z][a-z0-9-]{1,30}$`), `name`, `version` (semver), `api_version = 1`,
  `platforms` (any of `macos`, `linux`, `windows`), `permissions`.
- Unknown keys are errors. The JSON schema is `claude-garage plugin schema`; the `#:schema` first line
  gives editors autocomplete.
- Commands are argv arrays, never a shell string. Variables: `{plugin_dir}`, `{state_dir}`, `{platform}`.
  Use `["sh", "-c", "..."]` only when a shell is really needed.
- `[[actions]]`: `id`, `title`, `contexts` (`global`, `workspace`, `session`), and exactly one of
  `command` or `pane`. Optional `default_key` (never overrides an existing key), `confirm`, `timeout_secs`.
  The full action id is `<plugin id>:<action id>`.
- `[[events]]`: `on` is one of `session.created`, `session.closed`, `session.turn`, `session.forgotten`,
  `session.status_changed`, `worktree.created`, `workspace.added`, `workspace.removed`, `daemon.started`,
  or `timer` (needs `every_secs`, at least 5).
- `[[widgets]]`: `id` and `slot`: `strip.right`, `rail.header`, `rail.row`, `rail.extra_rows`,
  `tile.title.right`, `tile.gutter`.
- `[[panes]]`: `id`, `title`, `placement` (`tile`, `overlay`, `popup`), `command`. A pane is a program
  that draws in a terminal; garage hosts it like a session.
- `[[cli]]`: `claude-garage <name>` runs a command. `[[build]]`: commands run once at install and again on every `plugin dev` reload that follows a code change (not a manifest-only change), so keep them fast and safe to repeat.
- `permissions` are declared and shown to the user, not enforced. Ask only for what you use:
  `ui.widgets`, `ui.panes`, `ui.notice`, `ui.invoke`, `ui.navigate`, `sessions.read`, `workspaces.read`,
  `workspaces.write`, `sessions.manage`, `network`, and the dangerous ones, `sessions.input` and
  `core.actions`. Do not add a dangerous permission unless the feature cannot work without it.

## What a command receives

Environment: `GARAGE_API` (base URL), `GARAGE_PLUGIN_ID`, `GARAGE_PLUGIN_TOKEN`, `GARAGE_PLUGIN_STATE_DIR`,
and the run context as JSON in `GARAGE_CONTEXT` (or the file `GARAGE_CONTEXT_FILE` when it is large, and on
stdin). The context has `focus.workspace`, `focus.session.id`, `focus.session.dir`, `trigger`, and for
events `event.name` and `event.data`. Commands run with a cleared environment, so do not expect the
user's shell variables.

## Pushing widgets

`PUT $GARAGE_API/api/plugins/$GARAGE_PLUGIN_ID/widgets/<widget>` with `Authorization: Bearer $GARAGE_PLUGIN_TOKEN`:

```json
{"scopes": {"*": [{"segments": [{"text": "5h ", "style": {"fg": "dim"}},
                                {"text": "42%", "style": {"fg": "warn", "bold": true},
                                 "click": {"action": "@@ID@@:greet"}}]}]},
 "ttl_secs": 120}
```

- Scope key by slot: `*` for `strip.right`, the workspace name for `rail.header` and `rail.extra_rows`,
  the session id for `rail.row` and `tile.title.right`.
- A PUT replaces the whole widget. Limits: 16 items per scope, 16 segments per item, 64 columns of text per
  item, 16 KiB per request, 10 requests a second. Colours are theme tokens (`text`, `dim`, `accent`, `ok`,
  `warn`, `error`) or `#rrggbb`.
- Widgets live in the daemon's memory: push again on `daemon.started`.
- Keep the plugin quiet when it has nothing to say: clear the widget (`DELETE` the same URL) instead of
  showing an empty string.

## Rules

- Do not poll in a loop inside a command; commands have a timeout. Use a `timer` event for periodic work.
- Write only inside `$GARAGE_PLUGIN_STATE_DIR` (or the plugin folder while developing). Never touch the
  user's sessions, tmux or `~/.garage`.
- Print errors to stderr and exit non-zero; garage writes both streams to the plugin log.
- Add a test for every new function that decides something (parsing, counting, formatting). Do not test
  that a library works.
