# tui-wall (delta)

## REMOVED Requirements

### Requirement: Auto-subtitle in the tile bar
Superseded by "Title as display name" below — the title is promoted from
dim subtitle to primary name.

## ADDED Requirements

### Requirement: Title as display name
When a session's daemon-provided `title` is non-null and non-empty, the TUI
SHALL render it as the session's primary display name — in the tile bar (in
the label's position and styling, including the amber/focused/engaged
salience rules), the workspace rail row, and the triage queue row's
identity — with the auto-label demoted to a dim trailing id in the tile bar
only. When `title` is null or empty, the label SHALL render exactly as
before. In the tile bar's truncation ladder, glyph, branch, elapsed, the
frozen affordance, and the context meter never shrink; the display name
ellipsis-truncates into the remaining width, falling back to the untruncated
label when not even one character of the title fits; the trailing label id
is lowest priority and drops first.

#### Scenario: Title replaces the label as the name
- **WHEN** a session labeled `claude-1` has title "George employment history"
- **THEN** the tile bar, rail row, and queue row show "George employment history" as the name, and the tile bar shows `claude-1` dim after the bar content when it fits

#### Scenario: No title, no change
- **WHEN** a session's `title` is null
- **THEN** the tile bar, rail, and queue render the label exactly as before this change

#### Scenario: Long title truncates without pushing structure out
- **WHEN** a tile 40 cells wide shows a session whose title is longer than the space left by glyph/branch/elapsed/meter
- **THEN** the name renders ellipsis-truncated and glyph/branch/elapsed/meter are unaffected
