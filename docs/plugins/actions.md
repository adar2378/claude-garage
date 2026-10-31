# Actions and the confirm rule

An **action** is a command with a name. Users can bind it to a key, put it on a button, find it in the palette, or let a widget click run it.

```toml
[[actions]]
id = "refresh"                  # full id: usage-bar:refresh
title = "Refresh usage"         # shown in help and the palette
contexts = ["global"]           # global | workspace | session
command = ["{plugin_dir}/bin/usage", "refresh"]
default_key = "u"               # optional suggestion
confirm = false                 # see below
timeout_secs = 30               # 1 to 300, default 30
```

An action has exactly one of:

- `command`: run a program (no shell). See [commands and variables](manifest.md#commands-and-variables).
- `pane = "details"`: open one of the plugin's [panes](panes.md).

## Contexts

`contexts` says what must be focused for the action to be available:

| Context | Needs |
|---|---|
| `global` | nothing |
| `workspace` | a focused workspace |
| `session` | a focused session |

The run context then has `focus.workspace` or `focus.session` filled.

## Keys

`default_key` is a suggestion. It never takes a key that is already bound. If two plugins suggest the same key, the plugin installed first wins. Users bind or clear a key themselves:

```sh
claude-garage plugin bind usage-bar:refresh u
claude-garage plugin bind usage-bar:refresh --clear
```

Their choice is kept across updates.

## How an action runs

1. The user (or a widget click) triggers it. The wall sends the focus to the daemon.
2. The daemon builds the [run context](api.md#the-run-context). It fills in the directory, status, branch and worktree from its own state, and ignores what the wall claims about them.
3. The daemon starts your command with the [environment](api.md#environment). `trigger.kind` says why: `key`, `button` (a manifest button), `widget` (a click in a widget), or `palette`.
4. Exit code 0 is success. Anything else is a failure and the user sees a notice. The output goes to the log (`plugin logs`).

A run that passes `timeout_secs` is stopped (SIGTERM, then SIGKILL) together with its children.

## Triggering actions from your plugin

Three ways to make an action run besides a key:

- A [button](widgets.md#buttons) in the manifest.
- A `click` in a widget you push.
- `POST /api/plugins/ui/action` (needs `ui.invoke`):

```json
{"action": "session.focus", "target": {"session": "garage/app/fix"}, "args": {}}
```

`action` is a core id, `plugin:action` of your own, or a bare id of your own. `target` names a session that exists (live or with a closed record) and/or a registered workspace. Anything else is `422` and nothing reaches the wall. The reply is `202 {"ok": true}`: the daemon passed the request on, and the wall decides. A plugin gets 5 of these a second.

For a core id you need `ui.navigate` (focus and view ids) or `core.actions` (the rest). The allowed core ids are published as `features["actions.core_invocable"]`.

## The confirm rule

> A plugin can ask. Only the human can confirm.

Three parts:

1. **Dangerous core actions need a human second step.** `session.close`, `session.delete`, `session.restart`, `session.restore_all`, `workspace.restart`, `workspace.remove`, `workspace.remove.kill`, `daemon.restart` and `hooks.install` are dangerous. When a plugin trigger (a button, a widget click, `ui/action`) reaches one, the wall only **arms** it and shows a prompt. The user presses a key or clicks again to confirm. The request has no `confirmed` field, and the daemon never reads one.
2. **A manifest `confirm = true` adds a prompt.** On an action it makes plugin-origin triggers (a click, `ui/action`) ask first. On a button it adds a prompt on top of what a dangerous action already has. It can never remove a prompt.
3. **A key press is the user.** The user pressed the key, so an action with `confirm = true` still runs at once from its key. If your action does something the user should confirm, ask in your own pane or popup.

Some core ids cannot be triggered by plugins at all (`app.quit`, `overlay.close`, the rail toggles, and similar). And `session.engage` (move the keyboard into a session) is refused for `ui/action`: where the user types is the user's to change.

## Core ids a plugin may run

`GET /api/plugin/features` lists them in `actions.core_invocable`, plus the families `focus.cycle.*` and `overlay.open.*`. Navigation ids (no permission beyond `ui.navigate`): `jump.blocked`, `session.focus`, `workspace.focus`, `view.cycle`, `view.focus`.

Core ids contain a dot and no colon. Your own ids have no dot, so there is never a clash.
