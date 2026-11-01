---
title: "Introducing claude-garage 1.0: a terminal wall for Claude Code"
description: "claude-garage 1.0 is a terminal wall for Claude Code, now one native Rust binary with plugins and a Windows preview. What it is, what changed, how to try it."
date: 2026-11-01
tags: [release, rust, plugins]
draft: true
---

claude-garage is a terminal app that shows all of your Claude Code sessions on one screen and tells you which one is waiting for you. Version 1.0 is a rewrite as a single native binary written in Rust. This post says what it is, what changed, and what is still a preview.

*Status: this post is a draft until 1.0.0 is released. The current build is the release candidate 1.0.0-rc.1, published to npm under the `next` tag.*

## What is claude-garage?

claude-garage is a terminal wall for Claude Code. It shows one grid of live sessions per project. A session that waits for your answer turns amber, and one key (`a`) jumps to the session that has waited longest. tmux runs the sessions, so closing the terminal does not end them.

Everything stays on your machine. The background service listens on `127.0.0.1` only, and there is no account and no telemetry. It is MIT licensed.

## Why does the wall exist?

Running several Claude Code sessions means juggling terminal windows. The hard part is not the windows. It is knowing which session is blocked on you right now, and not losing sessions when you close a terminal or reboot.

So the wall does three things:

- **It shows every session at once.** One full-screen grid per project.
- **It tells you who needs you.** Amber marks a session that waits for input. `a` jumps to it from any project, and `A` lists all of them.
- **It keeps sessions alive.** After a reboot, stopped sessions show as restorable. `Enter` brings one back with its full conversation.

For a longer comparison with other ways of working, see [claude-garage vs many terminal tabs](/compare/many-terminal-tabs/).

## What changed in 1.0?

The biggest change is the shape of the program. In 0.6 there was a Node daemon and a Rust terminal wall. In the 1.0 release candidate, the daemon, the terminal wall and the `claude-garage` command are one Rust program. The [changelog](/docs/changelog/) states it plainly: there is no Node at runtime, and nothing else to download.

Other changes from the 1.0.0-rc.1 changelog:

- **One binary, several installs.** Use `npx claude-garage`, or a shell or PowerShell installer from the GitHub release. Prebuilt binaries exist for macOS (Apple silicon and Intel), Linux (x64 and arm64) and Windows (x64).
- **Automatic upgrade from 0.6.x.** The first `claude-garage` after the upgrade notices the old daemon, stops only that process, and starts the new one. No session is closed.
- **Plugins ship inside the binary.** The built-in plugins are embedded, and each plugin's files are checked against the hashes of that build before every start. This guards against accidental damage. It is not a defence against someone who already runs code as you, and the changelog says so.

The README gives the size: one 17 MB binary with the plugins inside it, about 6 MB of RAM and roughly 2% CPU.

## Is claude-garage extensible?

Yes, through plugins. A plugin is a folder with a `garage-plugin.toml` file and any program: a shell script, a Rust binary or a Node script. It can add a key, a button, a status widget, a panel, an event hook or a command line subcommand. Nine plugins are built in, such as a usage bar that shows your 5-hour and weekly usage, and a fuzzy jump palette.

Plugins run as you and are not sandboxed. garage shows every command and permission before install. A plugin can ask to close a session but only you can confirm it. The post on [the plugin confirm rule](/blog/plugin-confirm-rule/) explains that design. To write one, follow [Build your first plugin](/docs/guides/first-plugin/).

## What about Windows?

Windows x64 is a preview. It has a prebuilt binary and a PowerShell installer, but it has not been verified on real Windows yet. The changelog says that check is planned before 1.0.0. On Windows each session runs in a detached host process instead of tmux. Details are on the [Platforms page](/docs/platforms/).

## How do I try it?

You need macOS or Linux, tmux 3.2 or newer, and Claude Code on your `PATH`. Then:

```bash
npx claude-garage@next
```

Press `w` to add a project folder, `n` to start a session, `Enter` to type into it and `Ctrl+G` to get back to the wall. Press `?` for all keys. The [install guide](/docs/guides/install/) has the other install methods.

## What is next?

The README lists diff review in the terminal wall, adopting an existing tmux session, and phone notifications on the roadmap. Nothing there is promised for a date.

## Why did I build it?

I wanted a simple TUI to manage my Claude Code sessions across different projects. I tried a few similar apps before, but they were quite complex and did too much. So I made claude-garage: very simple, close to how the Claude desktop app feels, but in the terminal, with a flexible plugin architecture so anyone can add what they need. More about me and the project is on the [About page](/about/).

## Sources

- [claude-garage README](https://github.com/adar2378/claude-garage#readme) and [changelog](/docs/changelog/), version 1.0.0-rc.1.
- Claude Code docs: [CLI reference](https://code.claude.com/docs/en/cli-reference).

## Keep reading

- [Why tmux owns your Claude Code sessions](/blog/why-tmux-owns-your-sessions/).
- [How the claude-garage plugin rule keeps you in control of Claude Code](/blog/plugin-confirm-rule/).
- [Install claude-garage](/docs/guides/install/) or [try the wall in your browser](/try/).

claude-garage is not affiliated with Anthropic. "Claude" and "Claude Code" are Anthropic trademarks.
