# tui-context-meters Specification

## Purpose
TBD - created by archiving change p11-context-meters. Update Purpose after archive.
## Requirements
### Requirement: Tile context meter
When a session's `context` is non-null, its tile bar SHALL render a compact meter (4-segment bar + percentage, e.g. `▰▰▱▱ 42%`): dim/neutral below 80, red at 80 and above (red = "compact or restart soon" — a planning signal; amber remains exclusive to needs-input). Priority in the truncation ladder: above the subtitle, below glyph/label/branch/elapsed. Null context → bar renders exactly as today. The triage queue MAY show the percentage dim.

#### Scenario: Meter renders and thresholds
- **WHEN** sessions report 42% and 88%
- **THEN** the first meter renders dim `▰▰▱▱ 42%`, the second red `▰▰▰▰ 88%`; a session with null context shows no meter

### Requirement: Strip usage chip
When `GET /api/usage` has non-null data, the strip SHALL show a compact account chip (e.g. `5h 24% · wk 61%`), dim, refreshed at least every 60s, hidden entirely when both windows are null. It SHALL never displace the keys chip or notices.

#### Scenario: Chip appears with data
- **WHEN** usage reports fiveHour 24% and sevenDay 61%
- **THEN** the strip shows `5h 24% · wk 61%` dim; before any statusline post, no chip renders

### Requirement: Install affordance
When no session has statusline-sourced context and the hint was not dismissed this run, the wall MAY show a one-time dim strip hint pointing at the install action. An `install statusline` action (command via the help/overlay path or a key documented in help) SHALL call `POST /api/statusline/install` and report success/failure in the strip notice. The hint SHALL never be amber and never repeat after dismissal or success.

#### Scenario: Install from the wall
- **WHEN** the user triggers the install action and the daemon returns success
- **THEN** the strip notice confirms it and statusline-sourced meters appear once posts arrive

