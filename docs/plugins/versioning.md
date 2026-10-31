# Versioning

There are two version numbers. Do not mix them.

- The **garage version** (for example `0.7.0`) changes with every release.
- The **plugin API version** (`api_version = 1`) changes rarely. It names the whole contract between garage and plugins.

A plugin names the API version it was written for. garage loads it if it supports that version.

## What API v1 is

API v1 is **frozen**. These things are part of it, and are never removed or renamed within v1:

- manifest keys and their meaning
- permissions, and which are dangerous
- event names and payload keys
- slots, the widget format, theme tokens and limits (limits only grow)
- the routes in [api](api.md#routes), with the permission each needs
- the environment variables and the run context keys
- the core action ids plugins may run
- `features` keys of `GET /api/plugin/features`

`GET /api/plugin/features` says `"api.v1.frozen": true` and lists these sets. A test in the garage repository (`crates/plugin/api-v1.golden.json` with its tests) fails if any of them loses or changes an entry.

## The policy

**Additive within a major version.** garage may, in any release:

- add a manifest key (optional, with a default)
- add an optional request field or a response field
- add an event, an event payload key, a slot, a permission, a theme token, a core action id, a route
- raise a limit
- add a feature flag

garage will **not** within v1:

- remove or rename any of the above
- change what something means
- make an optional thing required
- lower a limit
- make an existing call need a permission it did not need before

So a plugin written for v1 keeps working on every later garage, as long as it **ignores what it does not know**: extra keys in the context, in event payloads and in replies.

## Using something new

When a garage release adds a feature, it also adds a feature flag or list entry. To use it:

- At run time: read `GET /api/plugin/features` and fall back if it is missing.
- To refuse to load without it, put it in `requires`:

```toml
requires = ["widgets.frame_x", "events:session.turn", "slots:tile.gutter"]
```

`name` needs a flag that is true or a list that is not empty. `name:value` needs `value` in the list. If garage lacks one, the plugin is not loaded and the message names what is missing. That is better than failing in the middle of a run.

## Breaking changes: API v2

A change that would break v1 needs `api_version = 2`. When that happens:

- garage loads v1 and v2 plugins side by side. A plugin that says `api_version = 1` keeps working unchanged.
- v1 stays supported for **at least six months and two minor garage releases after v2 ships**. This is the deprecation window.
- During the window `plugin list` marks v1 plugins "deprecated API" and the install preview says so. Release notes list what changed and how to move.
- After the window garage may stop loading v1. The message then says `uses removed plugin API v1 (this garage supports [2])`.

Going the other way: a plugin that says `api_version = 2` on a garage that only has v1 is not loaded, and the message says `needs a newer garage`.

## Your own plugin's version

`version` in the manifest is yours, and it is semver. garage shows it and passes it to your program (`GARAGE_PLUGIN_VERSION`). It does not interpret it.

## SDK versions

The Rust crate `garage-plugin-sdk` and the TypeScript package `@claude-garage/plugin-sdk` speak API v1. A new SDK release may add methods. It will not remove one within the same major SDK version.
