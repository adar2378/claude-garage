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
- the core action ids plugins may run (exact ids, listed one by one)
- the error body and its [codes](api.md#errors)
- `features` keys of `GET /api/plugins/features`

Every name in the list follows one rule: **dot-separated segments of lowercase snake_case** (`session.status_changed`, `widgets.click_args`, `needs_input`). That covers events, permissions, slots, theme tokens, features, limits, error codes and core action ids. Two things are not names in this sense: ids you choose yourself (plugin ids and CLI command names use hyphens, like `usage-bar`) and values such as session statuses (`needs-input`), which are data and keep their spelling.

`GET /api/plugins/features` says `"api.v1.frozen": true` and lists these sets. A test in the garage repository (`crates/plugin/api-v1.golden.json` with its tests) fails if any of them loses or changes an entry.

## The policy

**Additive within a major version.** garage may, in any release:

- add a manifest key (optional, with a default)
- add an optional request field or a response field
- add an event, an event payload key, a slot, a permission, a theme token, a core action id, a route
- raise a size limit
- add a feature flag
- add an error code

garage will **not** within v1:

- remove or rename any of the above
- change what something means
- make an optional thing required
- lower a size limit
- make an existing call need a permission it did not need before, except for a security fix (below)

So a plugin written for v1 keeps working on every later garage, as long as it **ignores what it does not know**: extra keys in the context, in event payloads and in replies.

### Limits: sizes and rates

Limits come in two kinds, and the golden file lists them apart (`limit_sizes`, `limit_rates`).

- **Sizes** (`widget_items`, `widget_segments`, `widget_cols`, `widget_body_bytes`, `widget_ttl_secs`, `widget_frames`, `notice_chars`) only grow in v1. A widget that fits today fits on every later garage.
- **Rates** (`widget_puts_per_sec`, and the per-second limits on notices, UI actions and slot updates) may be **tightened** in v1, to protect the user's terminal and machine. A plugin must handle `429` and `Retry-After` and must not depend on a particular rate.

### Security changes

A security fix may do these things inside v1, even though they are not additive:

- **Require authentication or a permission** on a call that did not need one. The routes that answer without a token today are marked below; plugins and the SDKs always send their token on them, so this breaks no plugin that follows the rule.
- **Lower a rate limit**, as above.
- **Move a permission into the dangerous set.** The permission keeps its name and meaning. The install preview marks it DANGEROUS, `--yes` stops granting it, and an update that already holds it asks again.

Nothing else is allowed under this heading. A security fix does not remove or rename a name, change what a call means, or lower a size limit. The release notes say when one of these changes lands, and which plugin calls could be affected.

### Routes that answer without a token

These routes work without a token today but **accept one**, and `features.routes` marks them `"tokenless": "deprecated"`:

`GET /api/events`, `GET /api/sessions`, `GET /api/workspaces`, `GET /api/usage`, `GET /api/plugins`, `GET /api/plugins/widgets`, `GET /api/plugins/panes`, `GET /api/workspaces/{name}/closed`.

Tokenless access to them is deprecated and may be removed in any release under the security rule above. Send `Authorization: Bearer $GARAGE_PLUGIN_TOKEN` on every call to them. The Rust and TypeScript SDKs always do. `GET /api/health` and `GET /api/plugins/features` stay open on purpose: a plugin needs them to find out whether the daemon is there and what it supports.

## Using something new

When a garage release adds a feature, it also adds a feature flag or list entry. To use it:

- At run time: read `GET /api/plugins/features` and fall back if it is missing.
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

The Rust crates `garage-plugin-sdk` and `garage-protocol` and the TypeScript package `@claude-garage/plugin-sdk` speak API v1. They have **their own versions, independent of the garage version**: `garage-plugin-sdk` and `garage-protocol` are 1.0.0, and a garage release that does not touch them does not bump them. Within 1.x, methods and fields are only added. The public structs and enums of the Rust crates are `#[non_exhaustive]`, with constructors and builders, so adding one does not break your build: construct with `Segment::new("x")`, not a struct literal, and match enums with a catch-all arm. Depend on them with `"1"` (any 1.x).

The TypeScript package needs Node 24 or newer.
