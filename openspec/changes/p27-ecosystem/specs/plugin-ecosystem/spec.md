## ADDED Requirements

### Requirement: Versioned plugin API
The daemon API used by plugins SHALL carry a major version. A plugin manifest SHALL declare the API version it needs and MAY list optional capabilities. garage SHALL refuse to load a plugin whose API version or capabilities it does not provide, with a message naming what is missing. A superseded API version SHALL keep working for a published deprecation window.

#### Scenario: Plugin needs a newer API
- **WHEN** a plugin declares `api = "2"` and garage provides only API v1
- **THEN** the plugin is not loaded and `claude-garage plugin list` shows it as disabled with "needs plugin API v2"

#### Scenario: Old plugin keeps working
- **WHEN** garage ships API v2 and a plugin declares `api = "1"` within the deprecation window
- **THEN** the plugin loads and works unchanged

### Requirement: SDKs generated from one schema
garage SHALL provide a Rust SDK and a TypeScript SDK for the plugin API, both generated from the same schema as the daemon's own types.

#### Scenario: SDKs match the daemon
- **WHEN** the API schema changes
- **THEN** regenerating the SDKs updates both, and the SDK contract tests run against a test daemon

### Requirement: Scaffold and dev loop
`claude-garage plugin new <name>` SHALL create a working plugin (manifest, one action, one widget, an authoring `CLAUDE.md`) in a chosen language. `claude-garage plugin dev <path>` SHALL link it, reload it when its files change, and stream its logs.

#### Scenario: New plugin runs immediately
- **WHEN** a user runs `claude-garage plugin new hello` and then `claude-garage plugin dev ./hello`
- **THEN** the plugin's action appears in garage and its widget renders without further setup

#### Scenario: Reload on change
- **WHEN** the user edits the plugin's widget text while `plugin dev` runs
- **THEN** the change appears in the wall without restarting garage

### Requirement: Discovery through a GitHub topic
A plugin SHALL be discoverable when its public repository contains `garage-plugin.toml` and carries the GitHub topic `claude-garage-plugin`. `claude-garage plugin search <words>` SHALL list matching repositories with name, description and stars.

#### Scenario: Search finds a published plugin
- **WHEN** an author adds the topic `claude-garage-plugin` to a public repo with a valid manifest
- **THEN** `claude-garage plugin search` with words from its description lists it, and `claude-garage plugin install owner/repo` installs it
