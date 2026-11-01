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
    "session": {"id": "garage/main", "dir": "/dev/garage", "status": "idle", "branch": "main", "worktree": null},
    "dir": null
  },
  "selection": {"session": "garage/main", "index": 0, "view": "wall"},
  "worktree": null,
  "event": null,
  "ui": {"slots": {"strip.right": {"width": 38}}, "visible": true, "terminal": "ghostty"},
  "garage": {"bin": "/usr/local/bin/claude-garage", "port": 4747, "dir": "/Users/me/.garage", "tmux_socket": null}
}
```

| Key | Meaning |
|---|---|
| `api_version` | `1` |
| `plugin` | You. `state_dir` is your storage folder. |
| `trigger` | `kind` is why you run (`key`, `button`, `widget`, `palette`, `event`, `timer`, `pane` or `cli`). `key` is the key for `key`. `slot` is the slot for `widget`. `action` is the full action id. `args` is the click or button `args` (`{}` if none). A CLI run has `{"argv": [...]}`. |
| `focus.workspace` | The focused workspace name. |
| `focus.session` | The focused session. The daemon fills `dir`, `status`, `branch` and `worktree` from its own state. It does not trust the wall for them. |
| `focus.dir` | CLI runs: the caller's working directory. `null` for every other run. |
| `selection` | What the user has selected in the wall. |
| `worktree` | The focused session's git worktree: `{path, branch, repo_dir}`. |
| `event` | Event runs only: `{name, at, data}`. See [events](events.md). |
| `ui.slots` | Geometry the wall reported for slots (`width`, `max_cols`, `filler`). May be empty. |
| `ui.visible` | `true` when a wall is in view: one reported itself visible within the last 45 seconds. A plugin that wants to alert the user (a sound, a banner) can skip the banner when this is `true`. Feature `ui.visible`. |
| `ui.terminal` | The terminal the wall runs in, as the wall reported it (it sends `__CFBundleIdentifier` when set, else `TERM_PROGRAM`): a bundle id such as `com.mitchellh.ghostty`, or a name such as `ghostty`. `null` until a wall has said. Only plain names (letters, digits, `.`, `-`, `_`) are kept. Feature `ui.terminal`. |
| `garage` | How to run `claude-garage` against this daemon: `bin` (absolute path or `null`), `port`, `dir` (`GARAGE_DIR`), `tmux_socket` (`GARAGE_TMUX_SOCKET` or `null`). Use it to build a command that comes back to garage, for example a click handler. Quote every value if you pass them to a shell. Feature `context.garage`. |

Ignore keys you do not know. New keys may be added in v1.

## Authentication

Send your token on every call: `Authorization: Bearer $GARAGE_PLUGIN_TOKEN`.

- `401`: no token, or not a valid one.
- `403`: the plugin is disabled, lacks the [permission](permissions.md) (the body names it), or the token belongs to another plugin than the one in the URL.
- Errors have the body described under [Errors](#errors).

Some read routes answer without a token today but accept one; always send it. See [versioning](versioning.md#routes-that-answer-without-a-token).

The daemon listens on `127.0.0.1` only. It refuses a request whose `Host` is not a loopback name, and any POST, PUT, PATCH or DELETE that carries a browser `Origin` header, so a web page cannot change anything.

## Routes

These routes are the API v1 contract. They are listed in `features.routes` of `GET /api/plugins/features`.

| Method and path | Needs | Purpose |
|---|---|---|
| `GET /api/plugins/features` | nothing | API versions and capabilities |
| `GET /api/health` | nothing | `{status, version, pid}` |
| `GET /api/events` | token accepted (tokenless deprecated) | Server-sent events stream of everything (see below) |
| `GET /api/sessions` | token accepted (tokenless deprecated) | Live and restorable sessions |
| `GET /api/workspaces` | token accepted (tokenless deprecated) | Registered workspaces |
| `GET /api/usage` | token accepted (tokenless deprecated) | 5 hour and weekly rate-limit windows |
| `GET /api/workspaces/{name}/closed` | `sessions.read` (checked when a token is sent; tokenless deprecated) | Closed chats of a workspace |
| `GET /api/plugins` | token accepted (tokenless deprecated) | Installed plugins, their actions, buttons, widgets and panes |
| `GET /api/plugins/widgets` | token accepted (tokenless deprecated) | Widget content of every plugin |
| `GET /api/plugins/panes` | token accepted (tokenless deprecated) | Open plugin panes |
| `PUT /api/plugins/{id}/widgets/{widget}` | `ui.widgets` | Set widget content. [Format](widgets.md) |
| `DELETE /api/plugins/{id}/widgets/{widget}` | `ui.widgets` | Clear a widget |
| `POST /api/plugins/{id}/notice` | `ui.notice` | Show a toast |
| `POST /api/plugins/ui/action` | `ui.invoke` | Ask the wall to run an action. [Actions](actions.md) |
| `POST /api/plugins/{id}/panes/{pane}/open` | `ui.panes` | Open a pane |
| `DELETE /api/plugins/{id}/panes/{pane}` | `ui.panes` | Close a pane |
| `POST /api/sessions/input` | `sessions.input` | Type into a session |

`{id}` in a path must be your own plugin id, or you get `403`.

Other routes exist (they run the wall and the CLI). They are **not** part of the contract and can change.

### Errors

Every non-2xx answer has this JSON body, and the keys are frozen:

```json
{"error": "plugin usage-bar lacks permission ui.widgets", "code": "permission_denied", "permission": "ui.widgets"}
```

- `error` is a sentence for people. It may be reworded in any release. Do not match on it.
- `code` is a stable name. Match on it. New codes may be added; handle an unknown one as the generic code of its HTTP status.
- `permission` is present only on `403` for a missing permission, and names it.

A route may add more keys (a `409` from `sessions/input` adds `status`). A `429` carries `Retry-After` in seconds.

| Code | Meaning |
|---|---|
| `bad_request` | 400 with nothing more specific to say. |
| `unauthorized` | 401 with nothing more specific to say. |
| `forbidden` | 403 with nothing more specific to say. |
| `not_found` | 404 with nothing more specific to say. |
| `conflict` | 409 with nothing more specific to say. |
| `payload_too_large` | The body or a field is over its size limit. |
| `invalid_request` | 422 with nothing more specific to say. |
| `rate_limited` | Too many calls; wait for `Retry-After` seconds. |
| `internal_error` | The daemon failed. |
| `not_implemented` | Not available in this build. |
| `unavailable` | The daemon cannot do this now. |
| `unspecified` | Any other status. |
| `invalid_json` | The body is not valid JSON. |
| `missing_token` | No `Authorization: Bearer` token. |
| `invalid_token` | The token is not a plugin's token. |
| `wrong_plugin` | The token belongs to another plugin. |
| `ui_token_required` | The route is for the wall; a plugin token is not enough. |
| `plugin_disabled` | The plugin is disabled. |
| `permission_denied` | The plugin lacks a permission; `permission` names it. |
| `unknown_plugin` | No such plugin. |
| `unknown_widget` | The plugin declares no such widget. |
| `invalid_widget` | The widget body is refused. |
| `unknown_action` | The plugin has no such action. |
| `action_not_allowed` | The action is not one a plugin may name. |
| `invalid_action` | `action` is missing or malformed. |
| `invalid_target` | `target` names something that does not exist. |
| `invalid_text` | `text` is missing or empty. |
| `unknown_pane` | The plugin declares no such pane. |
| `too_many_panes` | The plugin has too many panes open. |
| `pane_failed` | The pane could not be opened or closed. |
| `unknown_session` | No such session. |
| `session_not_allowed` | Not a garage session. |
| `invalid_if_status` | `if_status` is malformed. |
| `status_blocked` | The session is in a status `if_status` refuses; nothing was typed. |
| `send_failed` | The keys could not be sent. |
| `unknown_workspace` | No such workspace. |
| `host_not_allowed` | The `Host` header is not the daemon's. |
| `origin_not_allowed` | A browser origin tried to write. |
| `invalid_nonce` | The UI challenge nonce is malformed. |
| `no_ui_token` | This daemon has no UI token. |
| `bundled_plugin` | The plugin ships with garage and cannot be changed this way. |
| `reload_failed` | The plugin registry could not be reloaded. |
| `unknown_permission` | Not a plugin API v1 permission. |

### Notice

`POST /api/plugins/{id}/notice` with `{"text": "done", "ttl_ms": 3000}`. The text is cleaned and cut to 120 characters. `ttl_ms` is 1000 to 10000, default 5000. One a second, with a burst of three; more get `429`.

### Session input

`POST /api/sessions/input` with `{"id": "garage/app/fix", "text": "continue", "if_status": {"not": ["needs-input"]}}`.

It types `text` and presses Enter. Only `garage/...` sessions are accepted. `if_status.not` is checked at the moment the keys would go out: if the session is in one of those states the answer is `409` and nothing is typed. Use `needs-input` there (a session status value), so you never answer a permission question by accident. Needs the dangerous `sessions.input`.

### Events stream

`GET /api/events` is a server-sent events stream. Plugin events arrive as typed frames with the names in [events](events.md). Most plugins do not need it: an `[[events]]` hook is simpler and survives your program exiting.

## Versions and features

```
GET /api/plugins/features
{"api_versions": [1], "garage": "0.7.0", "features": {"api.v1.frozen": true, "events": [...], "slots": [...], "permissions": [...], ...}}
```

Use it to check for an optional capability at run time. To refuse to load without one, put it in `requires` in the manifest. See [versioning](versioning.md).
