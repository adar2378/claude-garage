# Events

An event hook runs a command when something happens in garage.

```toml
[[events]]
on = "session.closed"
command = ["{plugin_dir}/on-closed.sh"]
timeout_secs = 10      # optional: 1 to 60, default 10
```

The command gets the [run context](api.md#the-run-context). `trigger.kind` is `event`, and `event` holds the name, the time and the payload:

```json
{"name": "session.closed", "at": 1761900000000, "data": {"id": "garage/app/fix", "dir": "/dev/app", "workspace": "app", "reason": "user", "last_status": "idle"}}
```

`focus.workspace` and `focus.session` are filled from the event when they apply. For a session that is already gone the daemon uses what the event knew.

## Event names

| Event | When | Payload keys |
|---|---|---|
| `session.created` | A session appeared. | `id`, `dir`, `workspace` |
| `session.closed` | A session left the backend and its closed record is written. | `id`, `dir`, `workspace`, `reason` (`user`, `crash`, `daemon_restart`), `last_status` |
| `session.turn` | One Claude turn ended. | `id`, `workspace`, `at`, `transcript_offset`, `row`, `history_limit` |
| `session.forgotten` | A closed chat was deleted for good. | `id`, `workspace` |
| `session.status_changed` | A real status change. | `id`, `from`, `to`, `since` |
| `worktree.created` | garage made a git worktree for a session. | `workspace`, `session`, `path`, `branch`, `repo_dir` |
| `workspace.added` | A workspace was registered. | `name` |
| `workspace.removed` | A workspace was removed. | `name` |
| `daemon.started` | The daemon started. Widgets were forgotten; draw again. | none |
| `timer` | A schedule, not a bus event. Set `every_secs` (at least 5). | none |

Notes:

- Statuses are `idle`, `working`, `needs-input`, `done` and `restorable`.
- `row` of `session.turn` is the absolute scrollback row where the turn ended (0 is the oldest kept line). `transcript_offset`, `row` and `history_limit` can be `null` when the backend cannot say. Once a pane's history is full, old lines are trimmed and rows drift. The [turn-markers](../../plugins/turn-markers) plugin shows how to cope.
- `session.closed` comes after the closed record is written, so you can read it at once.
- Timestamps are Unix milliseconds.
- Ignore payload keys you do not know. New keys may be added in v1.

## How runs are scheduled

- Up to 4 runs of one plugin at the same time. More wait in a queue of 64. Past that the oldest queued run is dropped.
- A run that goes over its timeout gets SIGTERM, then SIGKILL, for the whole process tree.
- Because runs overlap, two runs can touch the same files. Use a lock around a read-change-write. See [storage](storage.md).
- **Five failed event runs in a row** switch the plugin's event hooks off. Reload the plugin to switch them on again. Runs started by the user (a key or a click) never count.
- If the daemon is busy with more than 256 event dispatches at once, new events are dropped with a log line instead of slowing everything.
- Events are not queued for a plugin while it is disabled.

To check what a plugin can use, read `features.events` from `GET /api/plugin/features`. In a manifest, `requires = ["events:session.turn"]` refuses to load on a garage that lacks it.
