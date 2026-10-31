---
title: Your first wall
description: Add a project, start Claude sessions, and learn the wall, the rail and the strip. Move between tiles, type into one, and get back to garage with Ctrl+G.
sidebar:
  order: 2
---

The wall shows every Claude session of a project at once. This page takes you from an empty wall to a working session.

## Steps

1. Run `claude-garage`.
2. Press `w` to add a project. Type the folder path, paste it, or drag the folder from Finder into the terminal. Press `Ctrl+O` to pick a folder in a dialog. Press `Enter` to add it. Press `Esc` to cancel.
3. Press `n` to start a Claude session in the project. A new tile appears.
4. Press `Enter` to type into the focused tile. Talk to Claude as you always do.
5. Press `Ctrl+G` to stop typing and go back to garage.
6. Press `?` at any time to see every key.
7. Press `q` to quit. Your sessions keep running.

## The three parts of the wall

- **The rail** is the list on the left. It shows your projects and the sessions in each one. Each session has a glyph for its state.
- **The grid** is the middle. It shows the sessions of the focused project as tiles. It shows at most 6 tiles. More sessions stay in the rail. Focus one, and it takes a grid slot.
- **The strip** is the bottom line. It shows project tabs, where your keys go (`keys -> garage`), and notices. A blocked project has an amber dot.

Session glyphs:

| Glyph | State | Meaning |
|---|---|---|
| `●` | needs input | Claude waits for your answer. Amber. Sorted first. |
| `◐` | working | Claude is running. |
| `✓` | done | Claude finished since you last looked. Green. |
| `○` | idle | Waiting for your next request. |
| `⟳` | restorable | The session stopped. Press `Enter` to bring it back. |

## Move around

| Key | Action |
|---|---|
| `1` to `9` | Switch project |
| `[` and `]` | Previous and next tile |
| `m` | Maximize the focused tile. Press `m` again to restore it. |
| `Tab` | Cycle the views of the project |
| `b` | Hide or show the rail |
| `<` and `>` | Make the rail narrower or wider |

You can also use the mouse. Click a rail row to focus it. Click a tile to type into it.

## Type into a tile and leave it

Garage has two modes. This is the most important idea on this page.

- **Garage mode.** Single keys control the wall. Typing letters does not reach Claude. The strip says `keys -> garage`.
- **Engaged mode.** Every key goes to one tile. The strip shows `keys -> project/session`.

To engage a tile, focus it and press `Enter`, or click it. To leave, press `Ctrl+G`. The `Ctrl+G` key is never sent to Claude. `Esc` is sent to Claude, so you can still use it to stop a turn.

Scroll back through a tile with `Shift+PageUp`. Press `End` to return to live output. The mouse wheel also scrolls.

## What you'll see

After `n`, a tile fades in and Claude Code starts inside it. When Claude finishes a turn, the glyph turns to `✓`. When Claude needs a decision, the tile turns amber. See [Many sessions](/docs/guides/many-sessions/) for that.

Press `I` once. It installs the Claude Code hooks (instant status) and the context meter into your Claude settings. Garage works without it, but status is faster with it. If it is already installed, the strip says so.

## Next

[Run many sessions at once](/docs/guides/many-sessions/)
