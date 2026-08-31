# Design: p10-views-and-subtitles

## Context

First feature change on the Rust wall (`wall/`). The web UI's `ui/src/lib/views.js` (pure, tested) is the semantic ancestor for views, extended from "named views" to the multi-group model the user described. Subtitles ride the daemon's existing `list-panes` call. Implementation is delegated to Sonnet agents with strict review; specs and harness greps are the contract.

## Goals / Non-Goals

**Goals:** views per the tui-views spec; auto-subtitles per the deltas; zero regressions (all `wall/test/e2e` suites stay green).
**Non-Goals:** explicit rename (`e`) — shelved; web UI adoption of views/title (later, free); drag-between-views; per-view custom layouts.

## Decisions

- **State model**: `views: Map<workspace, Vec<View{name, session_ids}>>` inside WallState; focused view per workspace; derived default view membership = sessions not claimed by a named view (mirrors views.js semantics — assignments are the exception list, the default is the complement; prunes automatically). Store transitions: `detach_focused`, `cycle_view`, `focus_view_of(session)` (used by jumps/rail clicks). Grid/LRU/cap logic becomes per-view (the existing gridded-set machinery keyed by (workspace, view)).
- **Persistence**: `~/.garage/wall.json` (GARAGE_DIR-aware) written debounced on view mutations; schema `{version, views: {ws: [{name, sessions}]}}`; load-or-default, prune on refetch. Client-side only — daemon stays authoritative for sessions, never for views.
- **Rendering**: view strip is one line above the grid (only ≥2 views); group frame = a Block border in the neutral border color around the grid area for multi-session views (inner tile borders unchanged); layout.rs gains the strip row + frame inset. Subtitle = dim span appended in the tile bar with lowest truncation priority.
- **Daemon title**: extend the existing `list-panes -F` format with `#{pane_title}`; normalize (empty/hostname/shell-name → null) in one small function with tests; carry through `sessions.js`. Poller cadence; no new tmux calls.
- **Keys**: `d` and `Tab` in the garage layer (both currently unbound; Tab-in-garage was reserved for exactly this). Help overlay updated. Harness-visible strings: view strip names, "detached <label>" strip notice — keep stable for e2e greps.
- **Delegation**: implementation waves run on Sonnet agents; review (me) gates each wave on: full test suites, clippy/npm clean, e2e re-runs, and spec-scenario spot checks.

## Risks / Trade-offs

- [Per-view grid state complicates LRU/cap] → keyed extension of the existing tested machinery, ported tests extended per view.
- [pane_title noise from shells] → the null-normalization list starts conservative (empty/hostname/login shell); a wrong subtitle is dim and harmless.
- [Sonnet implementation depth] → smaller waves, each with an explicit test contract; anything failing review twice escalates back to me.

## Open Questions

- None blocking.
