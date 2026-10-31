---
name: garage-plugin
description: Write a claude-garage plugin from a one-sentence request. Use when the user asks for a garage plugin, a widget in the garage rail or status strip, a garage action, hook, overlay or CLI subcommand.
---

# Write a claude-garage plugin

Turn the user's one-sentence request into a working plugin they can run with `claude-garage plugin dev`.
Do the work in a new folder; never touch `~/.garage`, tmux, or the user's sessions.

## Steps

1. **Decide the shape from the sentence.** Pick the smallest set of pieces:
   - something shown all the time: a **widget** (`strip.right` for one global item, `rail.header` per
     workspace, `rail.row` or `tile.title.right` per session)
   - something the user triggers: an **action** (optionally with `default_key`)
   - something that reacts: an **event** (`session.status_changed`, `session.closed`, `daemon.started`,
     `timer` with `every_secs` of at least 5, and so on)
   - a screen of its own: a **pane** with `placement = "overlay"`
   - a terminal command: **`[[cli]]`**
   Ask the user one question only if the sentence is ambiguous about something that changes permissions.
2. **Pick a language.** Shell for a few lines of glue that needs only `curl`; Rust when it parses files or
   keeps state; TypeScript when the user prefers Node. Default to what the user's project already uses,
   else shell for trivial and Rust otherwise.
3. **Scaffold.** From the folder where the plugin should live:
   `claude-garage plugin new <name> --lang <rust|ts|shell>`
   `<name>` is the id: lowercase letters, digits and `-`, 2 to 31 characters, starting with a letter.
4. **Read the scaffold's `CLAUDE.md`.** It is the authority on the manifest, slots, the widget format,
   permissions and limits. Follow it rather than memory.
5. **Edit the manifest and the code.** Replace the hello example; keep the file layout. Declare only the
   permissions the plugin uses (`ui.widgets`, `ui.panes`, `ui.notice`, `sessions.read`,
   `workspaces.read`, ...). Do not add `sessions.input` or `core.actions` unless the request cannot work
   without them, and say why in your reply.
6. **Write tests** for the functions that decide something (parsing, counting, formatting). Run them:
   `cargo test`, `npm test` or `sh test.sh`.
7. **Validate and run.** `claude-garage plugin dev .` checks the manifest (it prints every problem with the
   field path), asks the user to approve permissions and build commands, then watches the folder. Tell the
   user to run it themselves and to stop it with Ctrl+C; do not run it for them unless they ask.
8. **Report** what you made: the files, the permissions and why, how to trigger it, and how to publish
   (push to a public GitHub repo with the topic `claude-garage-plugin`).

## Rules

- Commands in the manifest are argv arrays, not shell strings.
- A widget PUT replaces the whole widget; push again on `daemon.started` because widgets live in memory.
- Never poll in a loop inside a command. Use a `timer` event.
- Write only inside `$GARAGE_PLUGIN_STATE_DIR` or the plugin folder.
- Plugins are not sandboxed; keep them small enough to read in one sitting.

## Example

Request: "show how many sessions need input in the status strip".

Result: `claude-garage plugin new needs-input --lang shell`, then an event hook on
`session.status_changed` and `daemon.started` that reads `GET $GARAGE_API/api/sessions` and PUTs the count
to a `strip.right` widget, with `permissions = ["ui.widgets", "sessions.read"]`, and a test that feeds a
fixed sessions list to the counting function.
