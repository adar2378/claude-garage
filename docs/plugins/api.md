# HTTP API, environment and run context

Everything a plugin does goes through three things: the environment it starts with, the JSON context it is given, and the daemon's HTTP API.

The wire types are published as a JSON schema: [`schemas/garage-plugin-api.schema.json`](../../schemas/garage-plugin-api.schema.json). The SDKs are built from it.

## Environment

garage starts a plugin with a **cleared** environment, plus these:

| Variable | Meaning |
|---|---|
| `GARAGE_API` | Base URL of the daemon, for example `http://127.0.0.1:4747` |
| `GARAGE_API_VERSION` | `1` |
| `GARAGE_PLUGIN_ID` | Your plugin id |
| `GARAGE_PLUGIN_VERSION` | Your plugin version |
| `GARAGE_PLUGIN_DIR` | Your plugin folder |
| `GARAGE_PLUGIN_STATE_DIR` | Your [state folder](storage.md) |
| `GARAGE_PLUGIN_TOKEN` | Your API token. Keep it private. |
| `GARAGE_TRIGGER` | Why you run: `key`, `button`, `widget`, `palette`, `event`, `timer`, `pane` or `cli` |
| `GARAGE_CONTEXT` | The run context as JSON. |
| `GARAGE_CONTEXT_FILE` | Instead of `GARAGE_CONTEXT` when the JSON is over 32 KiB: a private file with it. |
| `GARAGE_HANDOVER` | CLI runs only. See [cli](cli.md#handing-over-to-the-wall). |

Also kept from the user's environment: `PATH`, `HOME`, `USER`, `LANG`, `LC_*`, `TERM`, `TMPDIR`, `SHELL`. Nothing else. Your code does not see the user's API keys.

The run context is also on **standard input**.

The token is per plugin and per install. It is only valid while the plugin is enabled.

## The run context

Every action, event, pane and CLI run gets the same JSON. Keys are never missing; a value that does not apply is `null`. So you can index without checking.

```json
{
  "api_version": 1,
  "plugin": {"id": "usage-bar", "version": "0.1.0", "state_dir": "/Users/me/.garage/plugins/data/usage-bar"},
  "trigger": {"kind": "key", "key": "u", "slot": null, "action": "usage-bar:refresh", "args": {}},
  "focus": {
    "workspace": "garage",
    "session": {"id": "garage/main", "dir": "/dev/garage", "status": "idle", "branch": "main", "worktree": null}
  },
  "selection": {"session": "garage/main", "index": 0, "view": "wall"},
  "worktree": null,
  "event": null,
  "ui": {"slots": {"strip.right": {"width": 38}}}
}
```

| Key | Meaning |
|---|---|
| `api_version` | `1` |
| `plugin` | You. `state_dir` is your storage folder. |
| `trigger` | `kind` is why you run. `key` is the key for `key`. `slot` is the slot for `widget`. `action` is the full action id. `args` is the click or button `args` (`{}` if none). A CLI run has `{"argv": [...]}`. |
| `focus.workspace` | The focused workspace name. |
| `focus.session` | The focused session. The daemon fills `dir`, `status`, `branch` and `worktree` from its own state. It does not trust the wall for them. |
| `focus.dir` | CLI runs only: the caller's working directory. |
| `selection` | What the user has selected in the wall. |
| `worktree` | The focused session's git worktree: `{path, branch, repo_dir}`. |
| `event` | Event runs only: `{name, at, data}`. See [events](events.md). |
| `ui.slots` | Geometry the wall reported for slots (`width`, `max_cols`, `filler`). May be empty. |

Ignore keys you do not know. New keys may be added in v1.

## Authentication

Send your token on every call: `Authorization: Bearer $GARAGE_PLUGIN_TOKEN`.

- `401`: no token, or not a valid one.
- `403`: the plugin is disabled, lacks the [permission](permissions.md) (the message names it), or the token belongs to another plugin than the one in the URL.
- Errors have the body `{"error": "text"}`.

The daemon listens on `127.0.0.1` only. It refuses a request whose `Host` is not a loopback name, and any POST, PUT, PATCH or DELETE that carries a browser `Origin` header, so a web page cannot change anything.

## Routes

These routes are the API v1 contract. They are listed in `features.routes` of `GET /api/plugin/features`.

| Method and path | Needs | Purpose |
|---|---|---|
| `GET /api/plugin/features` | nothing | API versions and capabilities |
| `GET /api/health` | nothing | `{status, version, pid}` |
| `GET /api/events` | nothing | Server-sent events stream of everything (see below) |
| `GET /api/sessions` | nothing | Live and restorable sessions |
| `GET /api/workspaces` | nothing | Registered workspaces |
| `GET /api/usage` | nothing | 5 hour and weekly rate-limit windows |
| `GET /api/workspaces/{name}/closed` | `sessions.read` (if a token is sent) | Closed chats of a workspace |
| `GET /api/plugins` | nothing | Installed plugins, their actions, buttons, widgets and panes |
| `GET /api/plugins/widgets` | nothing | Widget content of every plugin |
| `GET /api/plugins/panes` | nothing | Open plugin panes |
| `PUT /api/plugins/{id}/widgets/{widget}` | `ui.widgets` | Set widget content. [Format](widgets.md) |
| `DELETE /api/plugins/{id}/widgets/{widget}` | `ui.widgets` | Clear a widget |
| `POST /api/plugins/{id}/notice` | `ui.notice` | Show a toast |
| `POST /api/plugins/ui/action` | `ui.invoke` | Ask the wall to run an action. [Actions](actions.md) |
| `POST /api/plugins/{id}/panes/{pane}/open` | `ui.panes` | Open a pane |
| `DELETE /api/plugins/{id}/panes/{pane}` | `ui.panes` | Close a pane |
| `POST /api/sessions/input` | `sessions.input` | Type into a session |

`{id}` in a path must be your own plugin id, or you get `403`.

Other routes exist (they run the wall and the CLI). They are **not** part of the contract and can change.

### Notice

`POST /api/plugins/{id}/notice` with `{"text": "done", "ttl_ms": 3000}`. The text is cleaned and cut to 120 characters. `ttl_ms` is 1000 to 10000, default 5000. One a second, with a burst of three; more get `429`.

### Session input

`POST /api/sessions/input` with `{"id": "garage/app/fix", "text": "continue", "if_status": {"not": ["needs-input"]}}`.

It types `text` and presses Enter. Only `garage/...` sessions are accepted. `if_status.not` is checked at the moment the keys would go out: if the session is in one of those states the answer is `409` and nothing is typed. Use `needs-input` there, so you never answer a permission question by accident. Needs the dangerous `sessions.input`.

### Events stream

`GET /api/events` is a server-sent events stream. Plugin events arrive as typed frames with the names in [events](events.md). Most plugins do not need it: an `[[events]]` hook is simpler and survives your program exiting.

## Versions and features

```
GET /api/plugin/features
{"api_versions": [1], "garage": "0.7.0", "features": {"api.v1.frozen": true, "events": [...], "slots": [...], "permissions": [...], ...}}
```

Use it to check for an optional capability at run time. To refuse to load without one, put it in `requires` in the manifest. See [versioning](versioning.md).
