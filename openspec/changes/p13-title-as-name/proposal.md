# Proposal: p13-title-as-name

## Why

`claude-1` means nothing to a user. p10's auto-subtitle put the meaningful
text (Claude Code's live OSC title, e.g. "George employment history") in the
*secondary* dim position while the meaningless auto-label kept the primary
spot. User feedback: flip it — the title IS the name.

## What Changes

- **Title becomes the primary display name** everywhere the TUI names a
  session (tile bar, rail, triage queue): when a session's `title` is
  non-null and non-empty, it renders where the label used to, in the label's
  styling; the auto-label (`claude-1`) demotes to a dim trailing id in the
  tile bar (dropped first when space is tight) and disappears from the rail
  and queue rows.
- **Fallback unchanged**: no title → the label renders exactly as before.
- **Truncation ladder (tile bar)**: glyph, branch, elapsed, frozen
  affordance, and context meter still never shrink; the primary name now
  ellipsis-truncates into the remaining width (it can no longer push other
  spans out); if not even one character fits, fall back to the untruncated
  label (old behavior). The trailing label id is lowest priority.
- Web wall: untouched (it never adopted `title`; it can adopt this later).

## Capabilities

### Modified Capabilities
- `tui-wall`: "Auto-subtitle in the tile bar" is superseded by
  "Title as display name".

## Impact

- Wall only: `wall_state.rs` (display_name), `tile.rs`, `rail.rs`,
  `triage.rs` (+unit tests). Daemon/API untouched.
- No breaking changes.
