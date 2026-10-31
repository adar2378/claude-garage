## ADDED Requirements

### Requirement: Plugin manifest
A plugin SHALL be a folder containing a `garage-plugin.toml` manifest with `id`, `name`, `version`, `api_version` and `platforms`, and MAY declare `[[actions]]`, `[[events]]`, `[[panes]]`, `[[widgets]]`, `[[buttons]]`, `[[cli]]` (optionally `fallback = true`), `[[build]]`, `[release]` and `permissions`. garage SHALL reject a manifest that fails validation against the published JSON schema and SHALL report which field failed.

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

#### Scenario: Preview states update policy
- **WHEN** garage prints an install preview
- **THEN** it states plainly that later updates changing only plugin code apply without a prompt

#### Scenario: Linked manifest adds permissions
- **WHEN** a linked plugin's manifest changes to add a permission
- **THEN** garage keeps running the last good manifest and shows "plugin X wants new permissions - run `claude-garage plugin link` to approve"

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
- **THEN** the plugin command runs once with the closed session in its event payload, including a `reason` of `user`, `crash` or `daemon-restart`

#### Scenario: Turn event
- **WHEN** Claude finishes a turn in a session (the Stop hook fires) and a plugin declares `[[events]] on = "session.turn"`
- **THEN** the plugin command runs once with the session id, a timestamp and the transcript offset when available

#### Scenario: CLI fallback
- **WHEN** a user runs `claude-garage ~/dev/app` and an installed plugin declares `[[cli]] fallback = true`
- **THEN** that plugin's command receives the unknown first word and the rest of the argv

#### Scenario: At most one fallback
- **WHEN** a second plugin declares `fallback = true`
- **THEN** garage refuses it and names the plugin that already holds the fallback, with a first-party plugin winning over a third-party one

#### Scenario: Install is not reserved
- **WHEN** a plugin declares `[[cli]] name = "install"`
- **THEN** the manifest is accepted, because core has no `install` subcommand

### Requirement: Plugin panes and widgets
`[[panes]]` SHALL run as backend sessions and show as a tile, overlay or popup. `[[widgets]]` SHALL render content the plugin pushes to a named slot through the daemon API; `[[buttons]]` SHALL render in a slot and trigger their `on_click` action. With no plugins installed, the TUI SHALL render exactly as before.

#### Scenario: Widget content appears
- **WHEN** a plugin pushes a styled segment to the `strip.right` slot
- **THEN** the strip shows that segment on the next frame

#### Scenario: Animated widget
- **WHEN** a plugin pushes a widget item with `frames` and `frame_ms`
- **THEN** the TUI cycles the frames itself with no further pushes, and the plugin pushes again only when its state changes

#### Scenario: Slot geometry available
- **WHEN** a plugin run starts or the terminal is resized
- **THEN** the plugin can read the strip width and filler range so a filler item can place itself

#### Scenario: Turn markers in the gutter
- **WHEN** a plugin pushes `tile.gutter` items for a session
- **THEN** the tile draws markers at those scrollback rows, keyed by that session

#### Scenario: Extra rail rows
- **WHEN** a plugin pushes `rail.extra_rows` items for a workspace
- **THEN** the rail shows those rows under that workspace with their style, and a click on a row runs its click target

#### Scenario: Widget click permission
- **WHEN** a widget PUT contains a `click.action` that is a core action id and the plugin lacks `core.actions`
- **THEN** the daemon rejects the PUT with 403 naming the permission

#### Scenario: Widget rev frame
- **WHEN** a plugin's widgets change
- **THEN** the SSE `widgets` frame carries `{plugin, rev}` and the TUI fetches only that plugin's widgets, and only when the rev differs from the one it holds

#### Scenario: Button triggers an action
- **WHEN** a plugin declares a button in `tile.title.right` with `on_click = "session.close"` and the user clicks it
- **THEN** that tile's session closes through the core action, with the plugin's `confirm` setting respected

### Requirement: Plugin API versioning and storage
Each plugin SHALL declare `api_version`; garage SHALL refuse to load a plugin whose major version it does not support and SHALL expose optional capabilities at `/api/plugin/features`. Each plugin SHALL get its own state directory.

#### Scenario: Unsupported API version
- **WHEN** a plugin declares an `api_version` newer than garage supports
- **THEN** garage does not load it and tells the user which garage version is needed

### Requirement: Click targets carry a payload on every plugin slot
Clickable items in `tile.title.right`, `rail.row`, `rail.header` and `strip.right` SHALL carry an action id and an optional JSON `args` payload, and a click SHALL run that action with the clicked item's session or workspace in its context, not the focused one. More than one filler item SHALL be allowed in `strip.right`.

#### Scenario: Button on an unfocused tile
- **WHEN** the user clicks a plugin button in the title bar of a tile that is not focused
- **THEN** the action runs with that tile's session in its context and the payload the plugin supplied

### Requirement: Dangerous actions need confirmation from plugins
A plugin button, key or UI request that targets a dangerous core action (close, restart, remove) SHALL NOT run it on the first trigger; garage SHALL ask for a second confirming trigger first, and a manifest SHALL NOT be able to remove that step. Core actions that are never safe to invoke from a plugin (`app.quit`, overlay-internal actions) SHALL be refused.

#### Scenario: Close needs a second click
- **WHEN** a plugin button bound to `session.close` is clicked once
- **THEN** garage asks for confirmation and the session closes only after the confirming click

### Requirement: Confirm rule for dangerous actions
Sources `Plugin` and `Widget` SHALL only ARM a dangerous action; only `Key`, `Mouse` or `Chord` sources SHALL CONFIRM it. A plugin that triggers a dangerous action twice SHALL NOT execute it.

#### Scenario: Plugin triggers twice
- **WHEN** a plugin sends `ui/action` for `session.close` twice in a row
- **THEN** the session is not closed; the action is armed and waits for a human key or click

#### Scenario: Human confirms
- **WHEN** a plugin armed `session.delete` and the user then presses the confirm key or clicks the armed region
- **THEN** the action runs once

### Requirement: Core action targets
Core actions invoked by plugins SHALL take their target only from the request `target` (`{session?, workspace?}`) or the click region, never from `args`. A click on a `strip.right` item that invokes a session-scoped core action SHALL target the focused session. `session.delete` SHALL forget a session (the `DELETE /api/sessions/{*id}?meta=1` semantics), be marked dangerous and be plugin-invocable with confirmation.

#### Scenario: Args cannot redirect
- **WHEN** a plugin invokes `session.close` with `target.session = "a"` and `args.session = "b"`
- **THEN** only session `a` is armed for closing

#### Scenario: Strip click targets focus
- **WHEN** the user clicks a `strip.right` item whose action is `session.close`
- **THEN** the focused session is the one armed for closing

### Requirement: Quick-send route
`POST /api/sessions/input` with body `{id, text}` SHALL send the text to the named session through `SessionBackend::send_text`. It SHALL require a valid plugin token and the `sessions.input` permission and SHALL be flagged dangerous.

#### Scenario: Missing token
- **WHEN** a caller posts to `/api/sessions/input` without a plugin token
- **THEN** the daemon answers 401 and sends nothing

### Requirement: Closed-session records
The core SHALL keep a bounded list of closed-session records per workspace and expose it through the API so plugins can render it.

#### Scenario: Closed chat listed
- **WHEN** a session closes
- **THEN** a record with its workspace, directory, closed time and reason appears in `GET /api/workspaces/{name}/closed` until restored or forgotten

### Requirement: Bundled first-party plugins
First-party plugins SHALL ship as Rust binaries in the package under `plugins/` and SHALL be seeded on the first daemon run with defaults: on `usage-bar`, `closed-chats`, `quick-send`, `tile-buttons`, `pit-pet`; off `jump-palette`, `turn-markers`, `cli-extras`, `prune-empty-projects`. No Node SHALL be needed at runtime.

#### Scenario: First run seeds defaults
- **WHEN** the daemon starts with no `plugins.json`
- **THEN** it registers the bundled plugins with the stated defaults enabled or disabled

#### Scenario: Disabled stays disabled
- **WHEN** the user disables a bundled plugin and garage restarts or upgrades
- **THEN** it stays disabled
