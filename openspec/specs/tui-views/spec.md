# tui-views Specification

## Purpose
TBD - created by archiving change p10-views-and-subtitles. Update Purpose after archive.
## Requirements
### Requirement: View partitioning
Each workspace's sessions SHALL partition into one or more named views; every session belongs to exactly one view of its workspace. A workspace starts with a single default view holding all sessions. The grid SHALL render only the focused view's sessions, with the 6-tile cap and overflow rules applying per view. Sessions arriving (spawn, restore, unregistered discovery) join the default view.

#### Scenario: New session joins the default view
- **WHEN** a session is spawned while the workspace has views "main" (default) and a solo view
- **THEN** the new session appears in "main" and the solo view is unchanged

### Requirement: Detach and rejoin
`d` in the garage layer SHALL move the focused session out of a multi-session view into its own solo view (named after the session label) and focus that view; `d` on a session that is alone in its view SHALL rejoin it to the default view and focus the default view. Empty non-default views SHALL be removed automatically.

#### Scenario: d detaches to a solo view
- **WHEN** the focused session sits in a 3-session view and the user presses `d`
- **THEN** the session renders alone (its own view focused), the origin view has 2 sessions, and pressing `d` again returns it to the default view

### Requirement: View strip and group frame
When a workspace has 2 or more views, a one-line view strip SHALL render above the grid naming each view (focused view emphasized; a view containing a needs-input session gets an amber dot). With a single view, no strip renders. A view containing 2 or more sessions SHALL render a subtle group frame around the grid area (neutral color, never amber); a solo view renders no frame — the frame's absence marks "individual". `Tab` in the garage layer SHALL cycle the focused workspace's views; the focused session follows into the newly focused view.

#### Scenario: Strip appears at two views
- **WHEN** a workspace gains a second view
- **THEN** the view strip renders with both names; when the workspace returns to one view the strip disappears

#### Scenario: Frame marks groups only
- **WHEN** the focused view holds 3 sessions
- **THEN** a neutral frame wraps the grid; switching to a solo view shows no frame

### Requirement: Salience is never trapped by views
Rail ordering, the strip badge, bell, and OSC title SHALL reflect blocked sessions regardless of which view they sit in. `a`-jump and triage-queue jumps SHALL switch workspace AND view as needed and land engaged. Focusing a session in a background view (rail click, jump) SHALL focus that session's view.

#### Scenario: a-jump crosses views
- **WHEN** a session in a background view of another workspace goes needs-input and the user presses `a`
- **THEN** the wall switches to that workspace and that view, engages the session, and the next keystroke reaches it

### Requirement: View persistence
View assignments and names SHALL persist across TUI restarts in a client-side file (`~/.garage/wall.json`, honoring GARAGE_DIR). The file is disposable: if missing or invalid, all sessions collapse into the default view with no error. Assignments for sessions that no longer exist SHALL be pruned on load and on refetch.

#### Scenario: Restart preserves groups
- **WHEN** the user detaches a session and restarts the TUI
- **THEN** the solo view is restored; deleting wall.json and restarting yields a single default view

### Requirement: Move to a group
`D` (shift+d) in the garage layer SHALL open a small picker overlay listing the focused workspace's views plus a "new group…" entry (which prompts for a name via the existing text-input overlay pattern). Selecting an entry SHALL move the focused session into that view and focus it. This is how multi-session named groups are built; `d` remains the quick detach/rejoin. Moving the last session out of a non-default view removes it.

#### Scenario: Building a three-session group
- **WHEN** the user focuses session A, presses `D`, creates group "backend", then moves sessions B and C into "backend" the same way
- **THEN** the workspace has views "main" and "backend"(A,B,C); "backend" renders with the group frame and its own grid

#### Scenario: Move-out removes empty views
- **WHEN** the only session of view "solo-x" is moved to "main" via `D`
- **THEN** "solo-x" no longer exists and the strip updates

