## ADDED Requirements

### Requirement: A first-party plugin can ship separately from the binary
A first-party plugin whose id is on the daemon's not-embedded list SHALL live in the repository's `plugins/` folder like the others and use only the public plugin API, but SHALL NOT be embedded in the `claude-garage` binary, SHALL NOT be on the compiled allowlist, SHALL NOT be seeded by `plugins::bundled`, and SHALL NOT be enabled by default. It is installed with `claude-garage plugin install <owner>/<repo>/plugins/<id>` after the normal install preview and grant, and updated with `claude-garage plugin update <id>` from its own releases. `openspec-board` SHALL be on that list, with `hello`. Every other first-party plugin SHALL remain embedded and allowlisted.

#### Scenario: Not in the release binary
- **WHEN** garage is built with `embed-plugins`
- **THEN** the embedded payload and the allowlist contain no entry for `openspec-board` or `hello`, and still contain every other first-party plugin

#### Scenario: Not seeded or enabled
- **WHEN** the daemon starts with no `plugins.json`
- **THEN** `openspec-board` is not registered until the user installs it

#### Scenario: Install shows permissions
- **WHEN** the user runs `claude-garage plugin install adar2378/claude-garage/plugins/openspec-board`
- **THEN** the preview lists its permissions and commands, none of them dangerous, and nothing runs until the user agrees
