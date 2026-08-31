# Proposal: p10-views-and-subtitles

## Why

With many sessions in one workspace, a single auto-balanced grid mixes unrelated work; users want terminals grouped ("backend" together, an experiment solo) — asked for repeatedly in user testing. Separately, auto-generated labels (`claude-1`) say nothing about what an agent is doing; Claude Code already broadcasts its activity via OSC terminal titles, which tmux captures per pane — a zero-config live subtitle.

## What Changes

- **Views (groups)**: each workspace's sessions partition into one or more named views; the grid renders one view at a time, wrapped in a subtle group frame (multi-session views only). `d` detaches the focused session into its own solo view / rejoins the default; `Tab` cycles views; a view strip appears above the grid when a workspace has ≥2 views. The 6-tile cap applies per view. Salience is never trapped: blocked sessions in background views still light the rail/badge/bell, and `a`-jump (and queue jumps) switch views to reach them. View assignments persist client-side (`~/.garage/wall.json`), disposable per the daemon-is-authoritative rule.
- **Auto-subtitles**: the daemon captures each session's tmux pane title (`#{pane_title}` — fed by Claude Code's OSC title updates) in its existing pane listing and exposes it as `title` on `GET /api/sessions` entries; the wall renders it as a dim truncated subtitle in the tile bar (and the queue) when it's meaningful (non-empty, not the default hostname/shell title). Explicit rename stays shelved.

## Capabilities

### New Capabilities
- `tui-views`: view partitioning, group frames, `d`/`Tab`, view strip, per-view grid cap, cross-view salience/jump, persistence.

### Modified Capabilities
- `session-status`: session listing entries additionally carry the live pane `title` (nullable) refreshed by the poller.
- `tui-wall`: tile bar renders the auto-subtitle; rail/queue may show it dim.

## Impact

- Daemon: one field through `tmux.js`/`sessions.js` (+tests). Web UI: untouched (it can adopt `title` later for free).
- Wall: state (view model + store transitions + persistence), UI (frame, strip, subtitle), e2e additions.
- No breaking changes; `wall.json` is new and disposable.
