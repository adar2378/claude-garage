---
title: Commands
description: The claude-garage command line, including restart options and plugin subcommands.
# TODO: hand-written from the README; generate when the CLI exposes a list.
sidebar:
  order: 2
---


| Command | What it does |
|---|---|
| `claude-garage` | Start the wall. Starts the background service if needed. |
| `claude-garage restart` | Restart only garage's background service. Sessions are not touched. |
| `claude-garage restart --sessions` | Restart every idle session on the latest `claude`. Conversations are kept. Busy sessions are skipped. |
| `claude-garage restart --sessions --all` | Restart busy sessions too. |
| `claude-garage daemon` | Run the background service in the foreground. |

Plugins can add their own subcommands. See [CLI commands and the fallback](/docs/plugins/cli/).
