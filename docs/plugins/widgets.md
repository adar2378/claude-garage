# Slots and widgets

A **slot** is a place in the wall. A **widget** is something your plugin puts in a slot. You declare the widget in the manifest, then push its content at run time over HTTP.

```toml
[[widgets]]
id = "bar"
slot = "strip.right"
```

Push content with `PUT /api/plugins/{plugin}/widgets/{widget}` (needs `ui.widgets`). The body replaces what the widget showed before.

```json widget:strip.right
{
  "scopes": {
    "*": [
      {
        "priority": 10,
        "segments": [
          {"text": "5h ", "style": {"fg": "dim"}},
          {"text": "42%", "style": {"fg": "warn", "bold": true},
           "click": {"action": "usage-bar:details", "args": {"window": "5h"}}}
        ]
      }
    ]
  },
  "ttl_secs": 120
}
```

The reply is `{"ok": true, "rev": 7}`. `rev` grows when the stored content changes. Pushing the same content again does not change it. `DELETE` on the same URL clears the widget.

## Slots

| Slot | Where it draws | Scope key |
|---|---|---|
| `tile.title.right` | The right end of a tile's title bar. | a session id |
| `tile.gutter` | The narrow column beside a tile's output, on the scrollback row you name. | a session id |
| `rail.row` | The end of a session's row in the left rail. | a session id |
| `rail.extra_rows` | Extra lines under a workspace in the rail. | a workspace name |
| `rail.header` | The workspace header row in the rail. | a workspace name |
| `strip.right` | The right side of the status strip at the bottom. | `"*"` (no scope) |

`scopes` maps a **scope key** to a list of **items**. A session id looks like `garage/app/fix`. The key `"*"` is the only one for `strip.right`. A key that matches nothing on screen is kept and ignored.

Each widget belongs to the slot you gave it in the manifest. A push is checked against that slot.

## Items

| Field | Meaning |
|---|---|
| `segments` | A list of text pieces drawn in a row. Each is `{text, style?, click?}`. |
| `frames` | Instead of `segments`: a list of frames, each a list of segments. See animation. |
| `frame_ms` | Milliseconds per frame: 100 to 2000, default 250. |
| `priority` | Higher wins when the space is narrow. Default 0. |
| `placement` | `{"kind": "flow"}` (default) or a filler. See below. |
| `click` | On a whole `rail.extra_rows` row or a gutter marker: what a click runs. |
| `row`, `kind` | `tile.gutter` only. `row` is required. See below. |
| `row_style` | `rail.extra_rows` only. A style for the whole row. |
| `session` | `rail.extra_rows` only. The session the row is about, as `garage/<workspace>/<label>` of that same workspace. |

`segments` and `frames` cannot both be set. An item needs one of them, except in `tile.gutter`.

### Style

`{"fg": ..., "bg": ..., "bold": true, "italic": true, "underline": true, "dim": true}`. All are optional.

`fg` and `bg` are a **theme token** or `#rrggbb`. Use tokens so your plugin follows the user's theme:

| Token | Use |
|---|---|
| `text` | normal text |
| `dim` | quiet text |
| `accent` | the garage accent colour |
| `ok` | good |
| `warn` | caution (not amber) |
| `error` | bad |
| `needs_input` | amber, but only while some session needs input. At other times it reads as plain text. |

### The needs_input token

Amber is the colour that means "a session is waiting for you". Plugins cannot paint it at will. If you want an alert that lights up only when something needs attention, use `needs_input`. The wall decides when it is lit.

### Click

```json
{"action": "usage-bar:details", "args": {"window": "5h"}}
```

`action` is a core action id, `plugin:action` of your own plugin, or a bare id of one of your own actions. A core id needs `ui.navigate` (navigation ids) or `core.actions`. See [actions](actions.md). `args` is at most 512 bytes of JSON and arrives as `trigger.args` in the action's [context](api.md#the-run-context). A click is checked when you push; a bad one gets `403` or `422`.

Segments in a row can each have their own click. An item-level `click` covers the whole row.

### Animation

Give `frames` instead of `segments`. The wall plays them as a loop on its own clock, so your plugin does not run again for each frame.

```json widget:strip.right
{
  "scopes": {"*": [{
    "frames": [
      [{"text": "( o.o )"}],
      [{"text": "( -.- )", "style": {"fg": "dim"}}]
    ],
    "frame_ms": 400
  }]},
  "ttl_secs": 15
}
```

At most 24 frames. There is no "play once". If you want a sequence, push a window of frames and push again before it ends.

### Filler (strip only)

A filler is drawn at a fixed column of the strip instead of in the flow. It is for a sprite that sits in the empty middle of the strip.

```json widget:strip.right
{
  "scopes": {"*": [{
    "placement": {"kind": "filler", "x": 4, "caption": "all quiet"},
    "frames": [[{"text": "^.^"}], [{"text": "^o^"}]],
    "frame_x": [4, 5]
  }]},
  "ttl_secs": 15
}
```

- `x` is the column from the left of the filler area. The wall tells you the area in `ui.slots` of the run context: `{"strip.right": {"width": 38, "max_cols": 38, "filler": {"x0": 10, "x1": 30}}}`.
- `caption` is text drawn beside the filler where there is room. Empty means none.
- `frame_x` gives one column per frame (same length as `frames`), so a sprite can walk. It replaces `x`.
- Filler is valid in `strip.right` only.

### Gutter markers

`tile.gutter` marks scrollback rows of a session's tile. Each item names an absolute `row` (0 is the oldest kept line, as in the `row` of `session.turn`).

```json widget:tile.gutter
{
  "scopes": {"garage/app/fix": [
    {"row": 1204, "kind": "marker", "click": {"action": "session.focus"}}
  ]}
}
```

`kind` is a label (`marker` is the usual one). With no `segments` the wall draws its default marker. A gutter takes up to 64 items per session.

### Extra rows

`rail.extra_rows` adds lines under a workspace. Use them for lists, such as closed chats. Up to 8 rows per workspace.

```json widget:rail.extra_rows
{
  "scopes": {"app": [{
    "segments": [{"text": "  fix login bug", "style": {"fg": "dim"}}],
    "session": "garage/app/fix",
    "click": {"action": "session.restore"}
  }]}
}
```

When `session` is set and the click is a core action such as `session.restore`, the action takes that session as its target.

## Limits

Limits are published as `features.limits` in `GET /api/plugins/features`. A push over a limit gets `413` (too big) or `422` (wrong shape), and the message names the limit.

| Limit | Value |
|---|---|
| Body size | 16 KiB |
| Scopes per widget | 256 |
| Items per scope | 16 (gutter 64, extra rows 8) |
| Segments per item | 16 |
| Display columns of text per item | 64 |
| Frames per item | 24 |
| `ttl_secs` | 1 to 3600 |
| Click `args` | 512 bytes |
| Pushes | 10 a second per plugin, burst of 20. Over it: `429` with `Retry-After`. |

Text is cleaned: control characters, line breaks and invisible direction marks are removed. Width is measured in display columns, so a wide character counts as 2.

## Lifetime

- Widgets live in the daemon's memory. **They are forgotten when the daemon restarts.** Listen to `daemon.started` and push again.
- `ttl_secs` removes the widget when it was not pushed again in time. Use a short one for live data (a pet uses 15). Then a crashed plugin does not leave stale text behind.
- Removing, disabling or reloading a plugin clears its widgets.

## Buttons

A button is a widget with no program. It is declared in the manifest and needs no push.

```toml
[[buttons]]
slot = "tile.title.right"
label = " x "
tooltip = "Close this session"
on_click = "session.close"
confirm = true
priority = 5
```

`on_click` is a core id, one of your actions, or a bare action id. `confirm = true` may only add a confirmation; it cannot remove the one a dangerous core action already has. A plugin with only buttons needs no program at all. See `plugins/tile-buttons`.
