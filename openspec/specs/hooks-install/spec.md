# hooks-install

## Purpose

One-click hook installation via the daemon (backup + atomic merge into ~/.claude/settings.json), replacing manual JSON merging as the primary path.
## Requirements
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

