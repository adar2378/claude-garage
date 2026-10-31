# Storage

Every plugin has one folder it may write to. garage creates it before the first run and keeps it when the plugin is updated or the daemon restarts.

```
~/.garage/plugins/data/<plugin-id>/      the state folder
~/.garage/plugins/logs/<plugin-id>.log   log of every run (stdout and stderr), 1 MiB, 2 files
```

`GARAGE_DIR` moves `~/.garage`.

## Finding it

- Environment: `GARAGE_PLUGIN_STATE_DIR`
- Run context: `plugin.state_dir`
- Command word: `{state_dir}` in the manifest

## Rules for good state

- **Treat it as a cache.** If a file is missing or damaged, work out the state again. Do not fail the run.
- **Write atomically.** Write a temporary file in the same folder, then rename it. A reader then never sees half a file. The Rust SDK does this in `write_state`.
- **Lock a read-change-write.** Up to 4 runs of one plugin can overlap, and they share the folder. Take a lock file around the whole step. The Rust SDK has `StateLock::acquire`. A lock older than a limit belongs to a dead run and is taken over.
- **Do not store secrets you can avoid.** The folder is readable by any program you run.
- **Do not write outside it.** Nothing stops you (see [security](security.md)), but a plugin that writes elsewhere cannot be cleaned up.

## What `plugin remove` does

`claude-garage plugin remove <id>` keeps the state folder and the log. `--purge` deletes them.

## Widgets are not storage

Widget content lives in the daemon's memory and is forgotten on restart. Keep what you need to redraw in your state folder, and redraw on `daemon.started`. See [widgets](widgets.md#lifetime).
