## ADDED Requirements

### Requirement: Plugin manifest
A plugin SHALL be a folder containing a `garage-plugin.toml` manifest with `id`, `name`, `version`, `api_version` and `platforms`, and MAY declare `[[actions]]`, `[[events]]`, `[[panes]]`, `[[widgets]]`, `[[buttons]]`, `[[cli]]`, `[[build]]`, `[release]` and `permissions`. garage SHALL reject a manifest that fails validation against the published JSON schema and SHALL report which field failed.

#### Scenario: Invalid manifest rejected
- **WHEN** a user installs a plugin whose manifest has an `[[actions]]` entry without a `command`
- **THEN** the install stops before running anything and names the missing field

### Requirement: Plugin install lifecycle
`claude-garage plugin install owner/repo[/subdir]` SHALL clone the repository, show the plugin's name, version, commands to be run and declared permissions, and register the plugin only after the user confirms. When the manifest lists a release binary for the current OS, garage SHALL download it instead of building. `plugin link <path>` SHALL register a local folder without building. `plugin update`, `plugin list` and `plugin remove` SHALL manage installed plugins.

#### Scenario: Install shows permissions first
- **WHEN** a user runs `claude-garage plugin install someone/usage-bar`
- **THEN** garage prints the plugin summary and permissions and waits for confirmation before any build, download or plugin command runs

#### Scenario: Prebuilt binary used
- **WHEN** the manifest lists a release asset for the current OS
- **THEN** garage downloads that asset and runs no `[[build]]` command

### Requirement: Declared permissions without a sandbox
Plugins SHALL declare permissions in their manifest. The install prompt SHALL list them and SHALL flag `sessions.input` (sending input to sessions) as dangerous. garage SHALL state that plugins run with the user's full rights and are not sandboxed.

#### Scenario: Dangerous permission flagged
- **WHEN** a plugin declares `sessions.input`
- **THEN** the install prompt marks it as dangerous and explains that the plugin can type into any session

### Requirement: Plugin actions, events and CLI commands
garage SHALL run plugin commands as separate processes without an intermediate shell. Each action, event and CLI invocation SHALL receive a context JSON with the focused session, workspace, directory, worktree, selection and trigger. Plugin actions SHALL be bindable to keys and buttons like core actions. `[[cli]]` entries SHALL be callable as `claude-garage <name>`.

#### Scenario: Action gets context
- **WHEN** the user triggers a plugin action while a session tile is focused
- **THEN** the plugin process receives that session's id, workspace and directory in its context JSON

#### Scenario: Event hook fires
- **WHEN** a session closes and a plugin declares `[[events]] on = "session.closed"`
- **THEN** the plugin command runs once with the closed session in its event payload

### Requirement: Plugin panes and widgets
`[[panes]]` SHALL run as backend sessions and show as a tile, overlay or popup. `[[widgets]]` SHALL render content the plugin pushes to a named slot through the daemon API; `[[buttons]]` SHALL render in a slot and trigger their `on_click` action. With no plugins installed, the TUI SHALL render exactly as before.

#### Scenario: Widget content appears
- **WHEN** a plugin pushes a styled segment to the `strip.right` slot
- **THEN** the strip shows that segment on the next frame

#### Scenario: Button triggers an action
- **WHEN** a plugin declares a button in `tile.title.right` with `on_click = "session.close"` and the user clicks it
- **THEN** that tile's session closes through the core action, with the plugin's `confirm` setting respected

### Requirement: Plugin API versioning and storage
Each plugin SHALL declare `api_version`; garage SHALL refuse to load a plugin whose major version it does not support and SHALL expose optional capabilities at `/api/plugin/features`. Each plugin SHALL get its own state directory.

#### Scenario: Unsupported API version
- **WHEN** a plugin declares an `api_version` newer than garage supports
- **THEN** garage does not load it and tells the user which garage version is needed
