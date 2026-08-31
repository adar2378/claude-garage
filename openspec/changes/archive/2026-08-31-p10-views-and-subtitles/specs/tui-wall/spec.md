# tui-wall (delta)

## ADDED Requirements

### Requirement: Auto-subtitle in the tile bar
When a session's daemon-provided `title` is non-null and differs from its label, the tile bar SHALL render it as a dim subtitle after the existing bar content, truncated with an ellipsis to the available width, lowest priority when space is tight (glyph, label, branch, elapsed win). The triage queue row MAY show the same subtitle dim when present. Subtitles are never amber and never replace the label.

#### Scenario: Subtitle renders and truncates
- **WHEN** a tile 40 cells wide shows a session whose title is longer than the space after label/branch/elapsed
- **THEN** the subtitle renders dim and ellipsis-truncated, and the label/branch/elapsed are unaffected

#### Scenario: No title, no change
- **WHEN** a session's `title` is null
- **THEN** the tile bar renders exactly as before this change
