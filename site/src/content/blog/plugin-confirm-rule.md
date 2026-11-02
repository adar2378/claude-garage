---
title: "The claude-garage plugin confirm rule"
description: "A claude-garage plugin can arm a dangerous action such as closing a session, but only a human key press or click confirms it. How the rule works."
date: 2026-11-01
tags: [plugins, security, design]
---

A claude-garage plugin can ask to close a session. It can never confirm. The wall only arms the action and shows a prompt, and you press a key or click a second time. This post explains the rule, the code-level reason it holds, and what it does not protect.

## What is the confirm rule?

The [plugin docs](/docs/plugins/actions/) state it in one line: a plugin can ask, only the human can confirm. The list of dangerous actions lives in [`core_actions.rs`](https://github.com/adar2378/claude-garage/blob/main/crates/plugin/src/core_actions.rs).

Some core actions are dangerous. The list is `session.close`, `session.delete`, `session.restart`, `session.restore_all`, `workspace.restart`, `workspace.remove`, `workspace.remove.kill`, `daemon.restart` and `hooks.install`. When a plugin triggers one of them, through a button, a widget click or the `ui/action` API, the wall does not run it. It arms it and shows a prompt. You must press a key or click again.

## Why can a plugin not just send "confirmed"?

Because that field does not exist. The request a plugin sends has no `confirmed` property, and the daemon never reads one, even if a plugin adds it. Confirmation lives only in the wall, where it comes from your keyboard or mouse. The decision is not something a plugin can pass in.

This is the same pattern the wall uses for its own dangerous keys. For example, `x x` closes a session and `d d` discards a worktree: the second press confirms.

## What other limits protect where I type?

Two more rules close the obvious gaps:

- **A plugin cannot move your keyboard.** `session.engage`, which moves typing into a session, is refused wherever a plugin names an action. Where you type is yours to change.
- **Typing into a session is a marked permission.** `sessions.input` is flagged DANGEROUS in the install preview. A plugin that uses it should send with the `if_status` guard so it never answers a permission question by accident.

Plugin actions also pass through limits: a plugin gets 5 `ui/action` requests a second, and a request must name a session or workspace that exists, or it fails with `422` before reaching the wall.

## How do permissions fit in?

A permission is a word in the manifest's `permissions` list. garage shows it before install, and the daemon checks it on each API call. A call without the right permission gets `403` with a message that names the missing one. Two permissions are dangerous: `sessions.input` and `core.actions`. Installing a plugin that has one needs `--allow-dangerous` in scripts, and an update that adds one asks again.

Use the least you need. A widget that only jumps to a blocked session needs `ui.navigate`, not `core.actions`.

## What does the rule not protect?

The [security model](/docs/plugins/security/) is direct about this, and so should this post be. **Plugins are not sandboxed.** A plugin is a program that runs as you. It can read your files, use the network, and call core routes outside the plugin API. The confirm rule controls what a plugin can make the wall do, not what the program can do on its own. `permissions` is a declaration that garage shows and checks on its own API. It is not a boundary.

garage also does not sign or review plugins, and finding one on GitHub by topic is not a recommendation. So:

- Read the install preview, especially the commands.
- Pin a tag: `claude-garage plugin install owner/repo --ref v1.2.0`.
- Stop one at once with `claude-garage plugin disable <id>`.

If you find a way for a plugin to confirm a dangerous action itself, the security page asks you to report it to the maintainers as a security issue.

## How do I write a plugin that respects the rule?

Do not fight it. If your action must be confirmed, mark it `confirm = true` in the manifest. That only adds a prompt for plugin-origin triggers, and a key press still runs it at once because a key press is you. If an action needs your sign-off beyond that, ask in your own pane.

The [Build your first plugin](/docs/guides/first-plugin/) walkthrough shows a complete example, and the [security model](/docs/plugins/security/) and [actions](/docs/plugins/actions/) pages cover the rule in full.

## Why did I build plugins this way?

I wanted claude-garage to stay very simple, with a flexible plugin architecture so anyone can add what they need. The confirm rule is what lets that stay safe to use: a plugin can add a button or a key, but the final yes on a dangerous action is always yours.

## Sources

- claude-garage source: [`crates/plugin/src/core_actions.rs`](https://github.com/adar2378/claude-garage/blob/main/crates/plugin/src/core_actions.rs) (the dangerous action list). Facts checked against version 1.0.1.
- Docs: [actions](/docs/plugins/actions/), [permissions](/docs/plugins/permissions/) and [security](/docs/plugins/security/).

## Keep reading

- [Why tmux owns your Claude Code sessions](/blog/why-tmux-owns-your-sessions/).
- [Use and install plugins](/docs/guides/plugins/), then [install claude-garage](/docs/guides/install/) or [try the wall in your browser](/try/).
