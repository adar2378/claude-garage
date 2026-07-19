## ADDED Requirements

### Requirement: Remove a workspace without touching sessions
The daemon SHALL expose `DELETE /api/workspaces/:name` which removes the workspace's registry entry and its sessions' resume metadata, and SHALL NOT create, kill, or mutate any tmux session. Unknown names SHALL be rejected with 404. The rail SHALL provide a remove control on registered workspace headers with a two-step confirmation; live sessions of a removed workspace SHALL remain running and visible as an unregistered group.

#### Scenario: Removing a workspace leaves its sessions running
- **WHEN** workspace `kowboy` has a live session `garage/kowboy/checkout` and `DELETE /api/workspaces/kowboy` is called
- **THEN** the daemon responds 204, `kowboy` is gone from the registry, and `tmux ls` still shows `garage/kowboy/checkout` running

#### Scenario: Unknown workspace rejected
- **WHEN** `DELETE /api/workspaces/never-existed` is called
- **THEN** the daemon responds 404

### Requirement: Native folder picker endpoint
The daemon SHALL expose `POST /api/pick-directory` which, on macOS, opens a native folder picker dialog on the daemon's host and, when the user chooses a folder, responds with `{dir}` containing the absolute path of the chosen folder.

#### Scenario: Choosing a folder returns its absolute path
- **WHEN** `POST /api/pick-directory` is called on macOS and the user chooses `/Users/dev/kowboy` in the native picker
- **THEN** the daemon responds with `{dir:"/Users/dev/kowboy"}`

### Requirement: Cancelling the picker is reported distinctly
When the user cancels the native folder picker without choosing a folder, `POST /api/pick-directory` SHALL respond with `{cancelled:true}` rather than an error status.

#### Scenario: Cancelling the picker
- **WHEN** `POST /api/pick-directory` is called and the user dismisses the native picker without choosing a folder
- **THEN** the daemon responds with `{cancelled:true}` and no `dir` is returned

### Requirement: Picker unavailable on non-macOS returns 501
On a non-darwin host, `POST /api/pick-directory` SHALL respond 501, since no native picker mechanism is implemented for that platform.

#### Scenario: Non-darwin host returns 501
- **WHEN** `POST /api/pick-directory` is called on a non-macOS host
- **THEN** the daemon responds 501 and does not attempt to open any picker

### Requirement: Pick-directory sits behind the Origin allowlist
`POST /api/pick-directory`, being a state-changing request that triggers a host-native dialog, SHALL be subject to the same Origin allowlist enforcement as other state-changing daemon endpoints: requests carrying an `Origin` header outside the UI allowlist SHALL be rejected with 403, while requests without an `Origin` header SHALL be allowed.

#### Scenario: Cross-site request to pick-directory blocked
- **WHEN** `POST /api/pick-directory` arrives with `Origin: http://evil.example`
- **THEN** the daemon responds 403 and no picker dialog is opened

### Requirement: Add-workspace flow uses the picker with an editable auto-derived name
The add-workspace UI flow SHALL, by default, invoke the native folder picker (`POST /api/pick-directory`). On a successful pick, it SHALL derive a proposed workspace name from the chosen folder's basename — lowercased, with runs of characters outside `[a-z0-9]` collapsed to a single `-`, and trimmed — and, if that derived name collides with an already-registered workspace name, append `-2`, `-3`, etc. until unique. This proposed name SHALL be presented pre-filled and editable before the user confirms registration.

#### Scenario: Name derived from a simple folder basename
- **WHEN** the user picks folder `/Users/dev/Kowboy Project` via the picker
- **THEN** the UI pre-fills the workspace name field with `kowboy-project`, editable before save

#### Scenario: Derived name collides with an existing workspace
- **WHEN** the user picks a folder whose derived name is `kowboy`, and a workspace named `kowboy` is already registered
- **THEN** the UI pre-fills the name field with `kowboy-2` (or the next available `-N` suffix), editable before save

#### Scenario: Proposed name is editable before saving
- **WHEN** the picker returns a folder and the UI pre-fills a derived name
- **THEN** the user can edit that name before confirming, and the edited name (not the derived one) is what gets registered

### Requirement: Manual path entry remains available as a fallback
The add-workspace UI SHALL continue to offer a manual path-entry fallback (typing an absolute path directly) alongside the picker-first flow, for use on non-macOS daemons, SSH-forwarded browsers, or when the picker is unavailable or cancelled.

#### Scenario: Manual entry fallback is reachable from the add-workspace flow
- **WHEN** the user opens the add-workspace flow
- **THEN** a "type a path instead" (or equivalent) fallback control is available, letting the user register a workspace by typing an absolute directory path without invoking the native picker

#### Scenario: Picker unavailability does not block adding a workspace
- **WHEN** the daemon runs on a non-darwin host and `POST /api/pick-directory` would respond 501
- **THEN** the user can still add a workspace via the manual path-entry fallback

### Requirement: Rename a workspace via PATCH /api/workspaces/:name
The daemon SHALL expose `PATCH /api/workspaces/:name` accepting `{name: <new-name>}`, which renames the workspace's registry entry from `:name` to `<new-name>`. `<new-name>` MUST match the same naming pattern as workspace registration (`[a-z0-9-]+`). In addition to the registry entry, the rename SHALL apply to: every live tmux session belonging to that workspace, renamed from `garage/<old>/<label>` to `garage/<new>/<label>`; and every resume-metadata key for that workspace's sessions, rewritten to reference `<new>` instead of `<old>`.

#### Scenario: Renaming updates the registry entry
- **WHEN** `PATCH /api/workspaces/kowboy` is called with `{name:"kowboy-v2"}` and `kowboy-v2` is not already registered
- **THEN** the workspace previously named `kowboy` is now listed as `kowboy-v2`, resolving to the same directory

#### Scenario: Renaming renames live tmux sessions
- **WHEN** workspace `kowboy` has a live tmux session `garage/kowboy/checkout` and `PATCH /api/workspaces/kowboy` is called with `{name:"kowboy-v2"}`
- **THEN** the tmux session is renamed to `garage/kowboy-v2/checkout`, and any client already attached to it (grid terminal, iTerm) remains attached without interruption

#### Scenario: Renaming rewrites resume metadata
- **WHEN** workspace `kowboy` has resume metadata recorded under keys referencing `garage/kowboy/*` and it is renamed to `kowboy-v2`
- **THEN** the corresponding resume-metadata keys are rewritten to reference `garage/kowboy-v2/*`, so a subsequent restore for those sessions resolves correctly under the new name

### Requirement: Rename validation and conflict responses
`PATCH /api/workspaces/:name` SHALL respond 409 when `<new-name>` is already registered as a different workspace, 400 when `<new-name>` fails the naming pattern, and 404 when `:name` does not identify a currently registered workspace. In each of these cases, no registry entry, tmux session, or resume-metadata key SHALL be changed.

#### Scenario: Renaming to an already-taken name is rejected
- **WHEN** workspaces `kowboy` and `garage-dev` are both registered and `PATCH /api/workspaces/kowboy` is called with `{name:"garage-dev"}`
- **THEN** the daemon responds 409 and `kowboy` remains registered under its original name with its sessions untouched

#### Scenario: Renaming to an invalid name is rejected
- **WHEN** `PATCH /api/workspaces/kowboy` is called with `{name:"Ko Wboy"}` (uppercase and a space)
- **THEN** the daemon responds 400 and no rename is applied

#### Scenario: Renaming an unknown workspace is rejected
- **WHEN** `PATCH /api/workspaces/ghost` is called with `{name:"ghost-v2"}` and no workspace named `ghost` is registered
- **THEN** the daemon responds 404 and no registry, tmux, or metadata changes occur

### Requirement: Rail nests workspaces by directory containment
The workspace rail SHALL render a workspace indented beneath another workspace when the nested workspace's directory is inside the other workspace's directory. When a workspace's directory is inside more than one other registered workspace's directory, it SHALL be nested under the deepest (most specific) such ancestor. This nesting SHALL be derived at render time from the flat name-to-directory registry — the registry itself SHALL remain a flat map with no stored parent/child relationship.

#### Scenario: A workspace nests under its containing workspace
- **WHEN** workspace `garage` is registered at `/Users/dev/garage` and workspace `garage-ui` is registered at `/Users/dev/garage/ui`
- **THEN** the rail renders `garage-ui` indented beneath `garage`

#### Scenario: Deepest containing workspace wins for multi-level nesting
- **WHEN** workspace `a` is registered at `/Users/dev/a`, workspace `b` at `/Users/dev/a/b`, and workspace `c` at `/Users/dev/a/b/c`
- **THEN** the rail renders `c` nested beneath `b`, and `b` nested beneath `a`, not `c` nested directly beneath `a`

#### Scenario: Unrelated workspace directories are not nested
- **WHEN** workspace `kowboy` is registered at `/Users/dev/kowboy` and workspace `garage-dev` is registered at `/Users/dev/garage-dev` (neither directory contains the other)
- **THEN** the rail renders both at the top level, with neither indented beneath the other

#### Scenario: Nesting is derived, not stored
- **WHEN** workspace `garage-ui` (nested under `garage` by directory containment) is later re-registered at a directory outside `/Users/dev/garage`
- **THEN** the rail no longer renders `garage-ui` nested beneath `garage`, reflecting the new directory relationship without any explicit un-nesting operation
