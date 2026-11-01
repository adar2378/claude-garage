## ADDED Requirements

### Requirement: Product features ship as first-party plugins
garage SHALL ship closed-chats, jump-palette, quick-send, tile-buttons, usage-bar, turn-markers, prune-empty-projects, cli-extras and pit-pet as plugins in the repository's `plugins/` folder. Each SHALL use only the public plugin API and the same manifest format as third-party plugins.

#### Scenario: Plugin uses only the public API
- **WHEN** a first-party plugin needs a capability the plugin API does not offer
- **THEN** the capability is added to the plugin host and documented, and the plugin uses it through the public API

#### Scenario: Disabling a plugin removes its feature
- **WHEN** the user disables the usage-bar plugin
- **THEN** the strip shows no usage bar and nothing else in the wall changes

### Requirement: Closed chats and recent list
The daemon SHALL keep a record of closed sessions. The closed-chats plugin SHALL show closed sessions as grey rows in the rail and SHALL open a recent list on `H`, from which a closed session can be reopened.

#### Scenario: Closed chat stays visible
- **WHEN** the user closes a session with the closed-chats plugin enabled
- **THEN** the session stays in its workspace's rail as a grey row

### Requirement: Usage bar
The usage-bar plugin SHALL show 5-hour and weekly usage in the strip and SHALL keep the last known figures in its plugin storage across daemon restarts.

#### Scenario: Usage survives a restart
- **WHEN** the daemon restarts
- **THEN** the usage bar shows the last known figures until new ones arrive

### Requirement: Pit pet as a plugin
The pit pet SHALL run as a first-party plugin with the behaviour defined in the `tui-pit-pet` capability, enabled by default.

#### Scenario: Pet unchanged after the move
- **WHEN** a user upgrades to the version where the pet is a plugin
- **THEN** the pet appears and behaves as before without any setup

### Requirement: Plugin API v1 freeze
Plugin API v1 SHALL be frozen only after every first-party plugin in this capability works on macOS and Linux. After the freeze, breaking changes SHALL require a new `api_version` major.

#### Scenario: Freeze waits for all plugins
- **WHEN** one first-party plugin still needs an API change
- **THEN** API v1 is not frozen
