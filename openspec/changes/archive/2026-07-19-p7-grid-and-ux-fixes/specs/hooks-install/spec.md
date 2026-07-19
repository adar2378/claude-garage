# hooks-install

## ADDED Requirements

### Requirement: One-click hook installation endpoint
The daemon SHALL expose `POST /api/hooks/install` which merges the garage hook entries (the same content served by `GET /api/hooks/snippet`) into `~/.claude/settings.json`. Before writing, the daemon SHALL save a timestamped backup of the existing file next to it. The merge SHALL be additive and idempotent: existing unrelated settings and hooks SHALL be preserved, and calling install when the garage hooks are already present SHALL succeed without duplicating entries. On any failure (unreadable JSON, write error) the daemon SHALL respond with an error message and SHALL NOT leave `settings.json` partially written.

#### Scenario: Install merges without clobbering existing settings
- **WHEN** `~/.claude/settings.json` already contains unrelated user settings and hooks, and `POST /api/hooks/install` is called
- **THEN** the file afterwards contains both the pre-existing entries and the garage hook entries, and a backup of the pre-install file exists alongside it

#### Scenario: Install is idempotent
- **WHEN** `POST /api/hooks/install` is called twice in a row
- **THEN** the second call succeeds and the garage hook entries appear exactly once in `settings.json`

#### Scenario: Corrupt settings file is refused safely
- **WHEN** `~/.claude/settings.json` exists but is not valid JSON and install is called
- **THEN** the daemon responds with an error identifying the problem and the original file is left byte-for-byte unchanged

### Requirement: Install affordance in the hooks banner
The hooks banner SHALL offer an "install hooks for me" control as the primary action, calling `POST /api/hooks/install` and showing success or the returned error inline in the banner; the snippet link SHALL remain as a secondary manual path. The banner SHALL NOT be shown while the session list is empty (the current vacuous-`every()` trigger), only once at least one session exists and no hook-originated status has been observed.

#### Scenario: One-click install from the banner
- **WHEN** the user clicks "install hooks for me" and the endpoint succeeds
- **THEN** the banner reports success inline (and subsequently dismisses); no manual JSON editing was required

#### Scenario: No banner on an empty pit wall
- **WHEN** the UI loads with zero sessions
- **THEN** the hooks banner is not rendered, regardless of the observed status mix
