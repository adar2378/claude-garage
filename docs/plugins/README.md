# claude-garage plugins

A plugin adds something to garage. It can add a key, a button, a widget, a pane, an event hook or a command line subcommand.

A plugin is a folder with a file named `garage-plugin.toml` and any program you like. It can be a shell script, a Rust binary, a Node script. garage runs the program and gives it information. The program can call garage back over a small HTTP API on your machine.

This is plugin API **version 1**. It is frozen: see [versioning](versioning.md).

## What a plugin can do

| You want to | Use | Page |
|---|---|---|
| Run a command on a key, a click or from the palette | `[[actions]]` | [actions](actions.md) |
| React when something happens | `[[events]]` | [events](events.md) |
| Show text or an animation in the wall | `[[widgets]]` | [widgets](widgets.md) |
| Add a clickable label | `[[buttons]]` | [widgets](widgets.md) |
| Show a full program in a tile, overlay or popup | `[[panes]]` | [panes](panes.md) |
| Add `claude-garage <word>` | `[[cli]]` | [cli](cli.md) |
| Keep files between runs | state folder | [storage](storage.md) |

## Pages

1. [Quick start](quick-start.md): a plugin in 10 minutes.
2. [Manifest reference](manifest.md): every key of `garage-plugin.toml`.
3. [Permissions](permissions.md): what to ask for, and the two dangerous ones.
4. [Events](events.md): names and payloads.
5. [Slots and widgets](widgets.md): where things show and the JSON format.
6. [Actions](actions.md): and the confirm rule.
7. [Panes](panes.md)
8. [CLI commands](cli.md): and the fallback.
9. [Storage](storage.md)
10. [HTTP API and environment](api.md): routes, variables, run context.
11. [Security model](security.md): no sandbox. Read this before you install a plugin.
12. [Versioning](versioning.md)

## SDKs

You do not need an SDK. Any language that can read an environment variable and make an HTTP request works.

- Rust: [`garage-plugin-sdk`](../../crates/plugin-sdk/README.md)
- TypeScript: [`@claude-garage/plugin-sdk`](../../sdk/ts/README.md)

## Examples

The plugins in [`plugins/`](../../plugins) ship with garage and use only this public API:
`hello` (smallest), `usage-bar` (timer and widget), `pit-pet` (animation), `turn-markers` (gutter),
`closed-chats` (extra rows), `quick-send` (pane and session input), `tile-buttons` (buttons only, no program),
`alerts` (event hook that plays sounds and shows banners itself, with a CLI command).

## Install and manage

```sh
claude-garage plugin link ./my-plugin      # develop: use a local folder
claude-garage plugin install owner/repo    # install from GitHub (shows a preview first)
claude-garage plugin list
claude-garage plugin logs my-plugin -f     # follow the log
claude-garage plugin disable my-plugin
claude-garage plugin schema                # JSON schema of garage-plugin.toml
```
