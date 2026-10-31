---
title: Install
description: Install claude-garage with npx or the shell installer. Requires tmux 3.2 or newer and Claude Code on your PATH.
sidebar:
  order: 1
---

## Requirements

- macOS or Linux. Windows x64 is a preview. See [Platforms](/docs/platforms/).
- [tmux](https://github.com/tmux/tmux) 3.2 or newer. garage offers to install it with `brew` if it is missing.
- [Claude Code](https://docs.claude.com/en/docs/claude-code) on your `PATH`.

## Install

Both methods download a prebuilt binary. Nothing is compiled.

```bash
npx claude-garage
```

This needs Node.js 20 or newer. Or use the shell installer:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/adar2378/claude-garage/releases/latest/download/claude-garage-installer.sh | sh
```

On Windows (preview), use `claude-garage-installer.ps1` from the same [release page](https://github.com/adar2378/claude-garage/releases/latest).

## Upgrade

Run `claude-garage` again. It replaces the old background service by itself and leaves your sessions alone.

<!-- TODO: confirm install commands against the current release before launch. -->
