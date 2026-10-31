> **Detailed design is written when the phase starts (after p21/p22).** This file records only the decisions already made.

## Decisions made

**1. The acceptance test.** Every "plugin" row in the proposal's map runs as a plugin through the public plugin API only, with no private hooks. If a plugin needs something the API lacks, the gap is added to the plugin host (p24), not worked around.

**2. Core owns data and lifecycle; plugins own opinions and presentation.** Example: the daemon keeps closed-session records (data), the closed-chats plugin decides how to show them.

**3. Core items stay core.** Draft and screenshot recovery needs to read every keystroke sent to a session; without a sandbox that is too much trust to hand a plugin. Selection, copy and sidebar layout are terminal fundamentals.

**4. First-party plugins live in the repo** (`plugins/<name>/`), ship with garage and are enabled by default where the feature existed before (pit pet). They use the same manifest and install path as third-party plugins.

**5. Freeze rule.** Plugin API v1 is frozen only when every plugin here works on macOS and Linux. Freezing earlier would freeze guesses.

## Open for the phase start
- Which plugins are enabled by default.
- Whether turn markers need a new scrollback slot or can use widgets.
- Source of truth for the usage figures (statusline only, or transcript fallback).

## Resolved (2026-10-31, user: all defaults)

- On by default: usage bar, closed chats, quick send, close/delete buttons, pet. Off by default: jump palette, turn markers.
- Two plugins in one slot: both render, in install order; a config setting can reorder them.
