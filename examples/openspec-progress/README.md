# OpenSpec progress

An example claude-garage plugin, and the reference for what a plugin built with `plugin new` and the
public SDK can do. It is **not bundled** with garage.

For every workspace whose folder has `openspec/changes/*/tasks.md`, it counts `- [x]` against `- [ ]`
and shows:

- a small `spec 12/30` widget in the **rail header** of that workspace (green when everything is done).
  Clicking it opens the overlay.
- an **overlay** (action `openspec-progress:changes`) listing every active change of the focused
  workspace with a bar, done/total and a percentage.

It re-reads the files every 5 seconds with a `timer` event, so ticking a box in your editor shows up
within a few seconds. There is no file watcher and no long-running process: each run reads a few small
files and exits.

Counting rules: list items (`-`, `*`, `+`) starting with `[ ]` or `[x]`/`[X]`, including indented
sub-tasks; checkboxes inside code fences are ignored; `openspec/changes/archive` and changes without
checkboxes are skipped.

## Permissions

`ui.widgets` (the rail-header widget) and `ui.panes` (the overlay). It learns each workspace's folder
from `GET /api/workspaces`, which needs no permission. It reads files inside those folders and nothing else. Plugins are not sandboxed;
read the source (about 300 lines in `src/lib.rs`) before you install anything.

## Try it

```sh
claude-garage plugin dev examples/openspec-progress   # from a clone of claude-garage
claude-garage plugin bind openspec-progress:changes O # optional: a key for the overlay
```

Or, once it lives in its own repository (see `Cargo.toml` for the SDK dependency to switch to):

```sh
claude-garage plugin install your-name/openspec-progress
```

## Test

```sh
cargo test        # no daemon, no network: a fake daemon records the requests
```

## Publish

Push it to a public repository with `garage-plugin.toml` at the root and add the GitHub topic
`claude-garage-plugin`; `claude-garage plugin search openspec` then lists it.
