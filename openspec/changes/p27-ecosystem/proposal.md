## Why

p24 gives garage a plugin host and p25 proves it with first-party plugins. Outside authors still need to find plugins, write one quickly, test it while they work, and trust that the API will not break under them. Without that, the plugin system only serves the core team.

## What Changes

- **Documented, versioned daemon API.** The HTTP API and event stream are published as API v1, with a schema. Plugins declare the API version they need; breaking changes go to v2 while v1 keeps working for a deprecation window.
- **SDKs.** Thin typed clients for the API: a Rust crate and a TypeScript package, generated from the same schema.
- **`claude-garage plugin new`** scaffolds a working plugin (manifest, one action, one widget) in Rust, TypeScript or shell.
- **`claude-garage plugin dev`** links a local plugin, reloads it when its files change, and streams its logs.
- **Discovery.** A plugin is published by adding the GitHub topic `claude-garage-plugin` to a public repo with a `garage-plugin.toml`. `claude-garage plugin search <words>` queries that topic. No registry server.
- **Docs.** A plugin section in the docs: concepts, manifest reference, API reference, slots and actions, permissions, examples.
- **Plugins written with Claude Code.** A plugin-authoring `CLAUDE.md` template in the scaffold, and a skill that writes a plugin from a one-sentence description.

## Capabilities

### New Capabilities
- `plugin-ecosystem`: API versioning promise, SDKs, scaffold and dev loop, discovery, authoring docs and template.

## Impact

- **Code:** schema export from the protocol crate; SDK packages; `plugin new`, `plugin dev`, `plugin search` subcommands; docs.
- **Depends on:** p24 (plugin host, manifest), p25 (API v1 frozen after first-party plugins exercise it).
- **No server.** Discovery reads GitHub; nothing for the project to host.
