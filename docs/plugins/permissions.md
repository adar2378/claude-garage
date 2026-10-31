# Permissions

A permission is a word in `permissions = [...]` in the manifest. It says what the plugin means to do.

garage does three things with permissions:

1. **It shows them** in the install preview, before anything runs.
2. **It checks them** on API calls. A call without the right permission gets `403` and a message that names the permission.
3. **It checks the manifest.** A manifest that uses a feature but does not declare its permission is refused. For example `[[widgets]]` needs `ui.widgets`.

Permissions are not a sandbox. A program you install can still read your files and use the network. See [security](security.md).

## The list

| Permission | Dangerous | What it allows |
|---|---|---|
| `sessions.read` | no | Read closed chats of a workspace (`GET /api/workspaces/{name}/closed`). Declare it if you read session data. |
| `sessions.input` | **yes** | Type text into a session (`POST /api/sessions/input`). |
| `sessions.manage` | no | Reserved. No v1 route checks it yet. |
| `workspaces.read` | no | Reserved. No v1 route checks it yet. |
| `workspaces.write` | no | Reserved. No v1 route checks it yet. |
| `ui.widgets` | no | Put and clear [widgets](widgets.md). Needed for any `[[widgets]]`. |
| `ui.panes` | no | Open and close [panes](panes.md). Needed for any `[[panes]]`. |
| `ui.invoke` | no | Ask the wall to run an action (`POST /api/plugins/ui/action`). |
| `ui.navigate` | no | Run only the navigation core actions: `jump.blocked`, `session.focus`, `workspace.focus`, `view.cycle`, `view.focus`. |
| `core.actions` | **yes** | Run any core action that plugins may run, in a button, a widget click or `ui/action`. |
| `ui.notice` | no | Show a short toast (`POST /api/plugins/{id}/notice`). |
| `network` | no | Reserved. Shown in the preview so users know the plugin talks to the network. |

"Reserved" means the word is valid and shown, but garage does not check it on a route. In v1 a later release may start checking a reserved permission only where doing so does not break a plugin that declared it.

## Dangerous permissions

Two permissions are marked **DANGEROUS** in the install preview:

- `sessions.input` can type into a session. A session is a Claude agent with your credentials and file access. Text typed there may be run.
- `core.actions` can close, delete or restart sessions and workspaces, and restart the daemon.

Because of this:

- `claude-garage plugin install --yes` does not grant them. Scripts must also pass `--allow-dangerous`.
- An update that adds a dangerous permission asks again.

## The confirm rule

Some core actions are dangerous: `session.close`, `session.delete`, `session.restart`, `session.restore_all`, `workspace.restart`, `workspace.remove`, `workspace.remove.kill`, `daemon.restart`, `hooks.install`.

A plugin can never confirm one. When a plugin triggers a dangerous action (a widget click, a button, `ui/action`), the wall only **arms** it. The user sees a prompt and must press a key or click a second time. The plugin cannot send "confirmed". The field does not exist in the API, and the daemon ignores it if sent.

A plugin also cannot move where the user types: `session.engage` is refused for `ui/action`, and the wall shows a notice if any plugin action ends up in a session.

More in [actions](actions.md).

## Ask for the least

- Widget only: `ui.widgets`.
- Widget with a click that opens your own pane: `ui.widgets`, `ui.panes`.
- A button that closes a tile: `core.actions` (dangerous, so the user will notice).
- A widget that jumps to a blocked session: `ui.navigate`, not `core.actions`.
- Answering prompts or sending text: `sessions.input`. Ask yourself first if you can avoid it. If you cannot, send with `if_status: {"not": ["needs-input"]}` so you never answer a permission question by accident.
