---
title: "claude-garage vs IDE agent panels"
description: "Compare claude-garage, a terminal wall for Claude Code, with agent panels inside an editor: sessions that outlive the window, plugins and trade-offs."
short: "A terminal wall versus an agent panel inside your editor. Where each one fits."
order: 4
otherLabel: "IDE agent panels"
intro: "An IDE agent panel puts an assistant inside your editor, next to your files and diffs. claude-garage is a terminal app that runs the real Claude Code CLI in tiles. They solve different problems and can be used together."
who:
  - "You already work in the terminal and run the Claude Code CLI."
  - "You want sessions that keep running when the editor or terminal closes."
  - "You work over SSH or on a machine without a graphical editor."
rows:
  - { topic: "Where it lives", garage: "A terminal. Works over SSH.", other: "Inside the editor window." }
  - { topic: "Editor integration", garage: "None. It does not see your editor's open files or selection.", other: "Strong. Panels can use open files, selections and inline diffs." }
  - { topic: "Reviewing changes", garage: "Diff review in the wall is on the roadmap, not shipped. Use git and your editor.", other: "Usually built in." }
  - { topic: "Many sessions at once", garage: "Built for it: a wall per project, amber for waiting, a to jump.", other: "Varies by panel. Often one conversation per panel or tab." }
  - { topic: "Surviving a closed window", garage: "Yes. tmux runs the sessions.", other: "Depends on the editor and the panel." }
  - { topic: "Extending it", garage: "Plugins in any language, shown in the rail, tiles and status strip.", other: "The editor's own extension system." }
  - { topic: "Privacy", garage: "The service listens on 127.0.0.1 only. No account, no telemetry.", other: "Depends on the panel." }
notNeeded:
  - "You live in your editor and one conversation at a time is enough."
  - "You rely on inline diffs and editor context for most tasks."
  - "You do not use the terminal."
faq:
  - q: "Can I use claude-garage and an editor panel together?"
    a: "Yes. They do not conflict. Use the wall to run and watch many sessions, and your editor to read code and review diffs."
  - q: "Does claude-garage run the same Claude Code as my editor?"
    a: "It runs the Claude Code CLI found on your PATH inside each tile. Your Claude settings, hooks and projects are untouched."
  - q: "Does claude-garage review diffs?"
    a: "Not yet. Diff review in the terminal wall is on the roadmap. For now, review changes with git and your editor."
  - q: "Can I use it from inside my editor's terminal?"
    a: "It is a terminal app, so it can run in any terminal that handles keys well. Ghostty, iTerm2, kitty and WezTerm work best. In macOS Terminal.app, Shift+Enter acts like Enter."
---

## Different tools for different jobs

An editor panel is close to the code. A wall is close to the sessions. If your main pain is "which of my five sessions is waiting for me", the wall answers it. If your main pain is "show me this change in context", the editor does.

Start with [Install](/docs/getting-started/install/) and the [first wall guide](/docs/guides/first-wall/) if you want to try both side by side.
