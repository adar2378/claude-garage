# Panes

A **pane** is a program with a screen. garage runs it in a terminal of its own and shows it in the wall. Use a pane when a widget is too small: a picker, a prompt, a log viewer.

```toml
permissions = ["ui.panes"]

[[panes]]
id = "details"
title = "Usage details"
placement = "overlay"          # tile | overlay | popup
command = ["{plugin_dir}/bin/usage", "ui"]
singleton = true               # default: opening again focuses the open pane
size = { cols = 80, rows = 24 }          # optional hint for overlay and popup
# size = { cols = 60, rows = 50, pct = true }   # percent of the screen

[[actions]]
id = "details"
title = "Usage details"
pane = "details"               # an action that opens the pane
default_key = "u"
```

## Placements

| Placement | Shows as |
|---|---|
| `tile` | A tile beside the agent tiles. |
| `overlay` | A large layer over the wall. |
| `popup` | A small box. |

The pane is the same kind of terminal as an agent tile, so sizing, mouse and scrollback work the same way.

## Life of a pane

- Opened by an action with `pane = "..."`, or by `POST /api/plugins/{id}/panes/{pane}/open` (needs `ui.panes`). The reply is `{"ok": true, "session": "plugin/<id>/<pane>", "reused": false}`.
- The program gets the same [environment and run context](api.md) as an action. `trigger.kind` is `pane`.
- It lives as long as its process. When the program exits, the pane closes. `DELETE /api/plugins/{id}/panes/{pane}` closes it from outside.
- A singleton pane is reused: opening it again returns `reused: true`.
- A plugin can have 4 panes open at once. The fifth gets `429`.
- Disabling or removing the plugin closes its panes.
- A pane is not an agent session. It never shows in the session list and never counts as "needs input".
- `GET /api/plugins/panes` lists the open panes.

## Writing the program

The pane's program draws with ordinary terminal control codes. Use any library. The Rust SDK has a small kit behind the `tui` feature (a fuzzy picker and a one-line prompt). See `plugins/jump-palette` for a picker and `plugins/quick-send` for a prompt that sends text into a session.

A pane that wants to do something to the wall calls the API: `ui/action` to focus a session, for example. Then it exits.
