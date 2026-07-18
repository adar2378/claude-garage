## ADDED Requirements

### Requirement: Open a workspace root in the editor
The daemon SHALL expose `POST /api/open-editor` accepting `{workspace}`, which resolves `workspace` via the workspace registry and opens that workspace's root directory in the configured editor CLI (e.g. `code <dir>`).

#### Scenario: Opening a workspace root
- **WHEN** `POST /api/open-editor` is called with `{workspace:"kowboy"}` and `kowboy` is registered at an existing directory
- **THEN** the daemon spawns the editor CLI against that directory and responds success

### Requirement: Open a specific file at a line
`POST /api/open-editor` SHALL also accept `{workspace, file, line}`, opening `file` (resolved relative to the workspace's registered directory) at `line` in the editor (e.g. `code --goto <file>:<line>`).

#### Scenario: Opening a file at a line
- **WHEN** `POST /api/open-editor` is called with `{workspace:"kowboy", file:"src/index.ts", line:42}`
- **THEN** the daemon spawns the editor CLI with a goto target resolving to `src/index.ts` at line `42` within `kowboy`'s registered directory

### Requirement: Path traversal guard
When `file` is supplied, the daemon SHALL resolve it against the workspace's registered directory and reject the request with 400 if the resolved path falls outside that directory (e.g. via `..` segments or an absolute path escaping the workspace root). The daemon SHALL NOT invoke the editor CLI for a rejected request.

#### Scenario: Traversal outside the workspace rejected
- **WHEN** `POST /api/open-editor` is called with `{workspace:"kowboy", file:"../../etc/passwd", line:1}`
- **THEN** the daemon responds 400 and does not spawn the editor CLI

#### Scenario: Absolute path escaping the workspace rejected
- **WHEN** `POST /api/open-editor` is called with `{workspace:"kowboy", file:"/etc/passwd", line:1}` where `/etc/passwd` is not inside `kowboy`'s registered directory
- **THEN** the daemon responds 400 and does not spawn the editor CLI

### Requirement: Unknown workspace returns 404
`POST /api/open-editor` SHALL respond 404 when `workspace` is not a registered workspace, whether opening the root or a specific file.

#### Scenario: Open-editor for unregistered workspace
- **WHEN** `POST /api/open-editor` is called with `{workspace:"ghost"}` and no workspace named `ghost` is registered
- **THEN** the daemon responds 404 and does not spawn the editor CLI

### Requirement: Editor CLI unavailable returns 501
When the configured editor CLI is not available on the host (e.g. `code` is not on `PATH`), `POST /api/open-editor` SHALL respond 501 with an error message the UI can surface to the user, rather than hanging or returning a generic 500.

#### Scenario: Missing editor CLI reported clearly
- **WHEN** `POST /api/open-editor` is called on a host where the editor CLI is not installed or not on `PATH`
- **THEN** the daemon responds 501 with an error body describing that the editor CLI is unavailable

### Requirement: Open-editor sits behind the Origin allowlist
`POST /api/open-editor`, being a state-changing request, SHALL be subject to the same Origin allowlist enforcement as other state-changing daemon endpoints: requests carrying an `Origin` header outside the UI allowlist SHALL be rejected with 403, while requests without an `Origin` header SHALL be allowed.

#### Scenario: Cross-site request to open-editor blocked
- **WHEN** `POST /api/open-editor` arrives with `Origin: http://evil.example`
- **THEN** the daemon responds 403 and does not spawn the editor CLI

### Requirement: UI affordances to open the editor
The UI SHALL provide a per-workspace control that opens that workspace's root in the editor, and an `o` keybinding that opens the currently selected file (in the changes pane or in review mode) at its currently viewed line.

#### Scenario: Per-workspace open-root control
- **WHEN** the user activates the open-root control for workspace `kowboy`
- **THEN** the UI calls `POST /api/open-editor` with `{workspace:"kowboy"}`

#### Scenario: o opens the current file in the changes pane
- **WHEN** the changes pane has a file selected and the user presses `o`
- **THEN** the UI calls `POST /api/open-editor` with `{workspace, file, line}` for the selected file

#### Scenario: o opens the current file in review mode
- **WHEN** review mode has a file selected and the user presses `o`
- **THEN** the UI calls `POST /api/open-editor` with `{workspace, file, line}` for the selected file

#### Scenario: o obeys terminal-focus suppression
- **WHEN** a terminal has input focus and the user types `o` as part of program input
- **THEN** `o` is sent to the pty and no open-editor call is made
