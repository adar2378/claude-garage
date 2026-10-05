## MODIFIED Requirements

### Requirement: Rail, strip, and salience ladder
The TUI SHALL render a workspace rail (workspaces with their sessions, glyphs `● ◐ ✓ ○ ⟳`) and a single-line bottom strip (workspace tabs with per-workspace needs-input dots, keys-target chip). Above the first workspace of every family of two or more workspaces (sharing a parent folder), the rail SHALL render one faint label row with the parent folder's name; single-workspace families get no label. Label rows SHALL NOT be numbered, SHALL NOT be clickable, and SHALL NOT change workspace numbering or the strip. Amber SHALL be used exclusively for `needs-input`: amber tile border and title, amber rail row, amber strip dot. `done` SHALL render green and stop being highlighted 2 minutes after the transition.

#### Scenario: Only blocked sessions are amber
- **WHEN** sessions in states needs-input, working, done, and idle are all visible
- **THEN** exactly the needs-input session renders amber (border, rail row); working/idle render in neutral tones; done renders green

#### Scenario: Done fades
- **WHEN** a session transitioned to `done` more than 2 minutes ago
- **THEN** its done highlight is no longer emphasized (glyph remains)

#### Scenario: Family label row
- **WHEN** workspaces `et-backend` and `et-admin` both live under `.../elite-traders/`
- **THEN** a faint `elite-traders` row renders above the first of them, workspaces keep their numbers, and clicking the label row does nothing while rows below it still hit the right targets
