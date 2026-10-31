---
title: Contributing
description: Build claude-garage from source with cargo, run the tests, and find your way around the project layout. How to send a change to the repository.
sidebar:
  order: 9
---

claude-garage is open source under the MIT license. This page shows how to build it from source, run its tests and find the code you want to change.

## What do I need to build claude-garage?

- Rust, stable channel. The repository pins `channel = "stable"` with rustfmt and clippy in `rust-toolchain.toml`. The minimum Rust version is 1.88, and the crates use edition 2024.
- tmux 3.2 or newer, for the tests that start real sessions.
- Claude Code on your `PATH`, if you want to run the wall.
- Node.js 20 or newer, only if you work on the TypeScript plugin SDK in `sdk/ts`.

## How do I build it?

```bash
git clone https://github.com/adar2378/claude-garage
cd claude-garage
cargo build
```

This builds the one binary: `target/debug/claude-garage`. The daemon, the terminal wall and the command line are all in it. A plain `cargo build` finds the first-party plugins in the `plugins/` folder. Release builds embed them with `--features bundle-plugins`.

Run it from the build folder:

```bash
./target/debug/claude-garage
```

## How do I run the tests?

```bash
cargo test -p garage-wall --lib --bins        # the terminal wall
cargo test -p garage-protocol -p garage-host -p garage-daemon -p claude-garage
```

Plain `cargo test` also starts and kills real tmux sessions. Run it in an environment where that is safe, and do not run it on a machine where you rely on live garage sessions.

Before you send a change, run clippy and rustfmt, which the toolchain file installs:

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

## What is the project layout?

| Path | What lives there |
|---|---|
| `wall/` | The terminal wall (ratatui). Keys, tiles, rail, overlays. The key help is in `wall/src/ui/help.rs`. |
| `crates/cli/` | The `claude-garage` command, the launcher, and the `plugin` subcommands, including the `plugin new` templates. |
| `crates/daemon/` | The background service: sessions, status, the HTTP API on `127.0.0.1`, the plugin runner, and the tmux backend. |
| `crates/host/` | The session host process used on Windows, or with `GARAGE_BACKEND=host`. |
| `crates/protocol/` | Shared wire types for the API. |
| `crates/plugin/` | The plugin manifest parser and checker. |
| `crates/plugin-sdk/` | The Rust SDK for plugin authors: `Api`, `Context`, widgets, a fake daemon for tests. |
| `plugins/` | The first-party plugins that ship inside the binary. |
| `sdk/ts/` | The TypeScript plugin SDK, `@claude-garage/plugin-sdk`. |
| `examples/openspec-progress/` | A full example plugin that is not bundled. |
| `schemas/` | The JSON schemas for the manifest and the plugin API. |
| `docs/plugins/` | The plugin documentation shown on this site. |
| `tests/contract/` | Contract tests. |
| `openspec/` | The specs and change records. |

The three layers are tmux, then the daemon, then the wall. See [why tmux owns your sessions](/blog/why-tmux-owns-your-sessions/) for the reason.

## How do I work on a plugin?

You do not need to build garage. Use `claude-garage plugin new` and follow [Build your first plugin](/docs/guides/first-plugin/). If you change the manifest format or the plugin API, the generated tables in `docs/plugins/manifest.md` come from the schema. Regenerate them with `UPDATE_DOCS=1 cargo test -p garage-plugin`.

Plugin API v1 is frozen. Names and routes in it are not removed or renamed within v1. See [versioning](/docs/plugins/versioning/).

## How are releases made?

A tag that starts with `v` triggers the release workflow. It builds the binaries with [cargo-dist](https://github.com/axodotdev/cargo-dist) for macOS (Apple silicon and Intel), Linux (x64 and arm64) and Windows (x64), and publishes the npm package. `dist plan` shows what a tag would publish. Maintainers do this, not contributors.

## How do I send a change?

1. Open an issue first for anything big, so we can agree on the shape.
2. Fork the repository, make a branch and keep the change small.
3. Add or update a test that proves the behavior. Tests should earn their place.
4. Run the tests, `cargo fmt` and clippy.
5. Open a pull request on [GitHub](https://github.com/adar2378/claude-garage) and say what changed and why.

Windows is a preview. A contributor who can verify the Windows build on real Windows is especially welcome. The [Platforms page](/docs/platforms/) lists the checks to do by hand.
