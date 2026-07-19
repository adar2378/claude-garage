# pit-wall-ui

## Purpose

The pit wall: workspace rail with needs-you-first ordering, simultaneous multi-terminal grid for the focused workspace, and chrome keybindings that coexist with typing into live terminals.

## Requirements

### Requirement: Workspace rail with nested sessions and status glyphs
The UI SHALL render a workspace rail listing every registered workspace, with that workspace's sessions nested beneath it. Each session SHALL display a status glyph corresponding to its status: `●` for `needs-input`, `◐` for `working`, `✓` for `done`, `○` for `idle`.

#### Scenario: Rail shows workspaces and nested sessions
- **WHEN** workspaces `kowboy` (sessions `checkout` status `needs-input`, `build` status `working`) and `garage-dev` (session `main` status `idle`) exist
- **THEN** the workspace rail shows both workspace names, each with its sessions nested underneath, and `checkout` renders `●`, `build` renders `◐`, `main` renders `○`

#### Scenario: Status glyph updates live
- **WHEN** a rendered session's status changes from `working` to `done` via the status push channel
- **THEN** the rail updates that session's glyph from `◐` to `✓` without a page reload

### Requirement: Needs-you-first ordering
The workspace rail SHALL order workspaces so that any workspace containing at least one session with status `needs-input` appears above workspaces with no such session. Within a workspace's session list, sessions with status `needs-input` SHALL be listed before sessions with any other status.

#### Scenario: Blocked workspace bubbles to the top
- **WHEN** workspace `alpha` (all sessions `idle`) was registered before workspace `beta` (one session `needs-input`)
- **THEN** the rail lists `beta` above `alpha`, despite `alpha` having been registered first

#### Scenario: Blocked session listed first within its workspace
- **WHEN** workspace `kowboy` has sessions `build` (`working`) and `checkout` (`needs-input`), with `build` having been created first
- **THEN** `checkout` is listed before `build` within the `kowboy` group

### Requirement: Simultaneous multi-terminal grid for the focused workspace
The terminal grid SHALL render every session belonging to the currently focused workspace as its own live, interactive xterm.js terminal, connected concurrently via `WS /term/:id`, stacked in the grid at the same time. A user SHALL be able to observe output streaming in more than one of these terminals simultaneously and interact with any of them, without switching windows, tabs, or navigating away from the grid.

#### Scenario: Two terminals of one workspace stream simultaneously with zero window switching
- **WHEN** the focused workspace `kowboy` has two sessions `checkout` and `build`, both actively producing output (e.g. Claude Code streaming a response in each)
- **THEN** both `checkout`'s and `build`'s terminals are visibly rendering and updating with their respective live output on screen at the same time, in the same view, with no window, tab, or route change required to see either

#### Scenario: Interacting with one terminal does not disturb the other
- **WHEN** both terminals of the focused workspace are visible and the user types into `checkout`'s terminal
- **THEN** the keystrokes are sent only to `checkout`'s pty and `build`'s terminal continues rendering its own independent output unaffected

#### Scenario: Grid updates when focus moves to a different workspace
- **WHEN** the user switches focus from workspace `kowboy` to workspace `garage-dev`
- **THEN** the grid now renders all of `garage-dev`'s sessions as live terminals, and `kowboy`'s terminal connections are no longer displayed in the grid

### Requirement: Focused-terminal highlight
Exactly one terminal within the grid SHALL be marked as focused at a time, visually distinguished (e.g. highlighted border) from the other terminals in the grid. Keyboard input not addressed to a specific terminal (e.g. typed characters) SHALL route to the focused terminal.

#### Scenario: Focused terminal is visually distinct
- **WHEN** the grid shows three terminals for the focused workspace
- **THEN** exactly one of the three has the focused-highlight styling applied, and the other two do not

#### Scenario: Clicking a terminal focuses it
- **WHEN** the user clicks on a non-focused terminal in the grid
- **THEN** that terminal becomes the focused terminal (gains the highlight) and the previously focused terminal loses it

### Requirement: Pit wall keybindings
The UI SHALL support the following global keybindings: `1`–`9` switch the focused workspace to the Nth workspace in rail order; `[` and `]` cycle the focused terminal to the previous/next session within the current workspace's grid; `a` jumps focus (workspace and terminal) to a session with status `needs-input`, searching across all workspaces, not only the currently focused one.

#### Scenario: Number key switches workspace
- **WHEN** the rail shows `kowboy` as the 2nd workspace and the user presses `2`
- **THEN** the focused workspace becomes `kowboy` and its grid renders

#### Scenario: Bracket keys cycle terminal focus within the workspace
- **WHEN** the focused workspace's grid has sessions `checkout` (focused) and `build`, and the user presses `]`
- **THEN** focus moves to `build`'s terminal

#### Scenario: `a` jumps to a blocked session in another workspace
- **WHEN** the focused workspace is `garage-dev` (no `needs-input` sessions) and workspace `kowboy` has a session `checkout` with status `needs-input`
- **THEN** pressing `a` switches the focused workspace to `kowboy` and focuses `checkout`'s terminal

### Requirement: Keybindings suppressed while typing into a terminal
Global pit-wall keybindings (`1`–`9`, `[`, `]`, `a`) SHALL NOT fire while keyboard focus is inside a terminal's input (i.e. the user is typing to send input to the running Claude Code process). Keystrokes in that state SHALL be delivered to the terminal's pty instead. The one exception is a dedicated chord, `Ctrl+\``, which SHALL blur the focused terminal and return keyboard focus to chrome-navigation mode even while the terminal has focus — it SHALL fire regardless of terminal focus, unlike every other global keybinding.

#### Scenario: Digit typed into terminal does not switch workspace
- **WHEN** a terminal has input focus and the user types `2` as part of a prompt to Claude
- **THEN** the `2` is sent to that terminal's pty as input, and the focused workspace does not change

#### Scenario: Bracket typed into terminal does not cycle focus
- **WHEN** a terminal has input focus and the user types `[` as part of program input
- **THEN** the `[` is sent to the pty and the focused terminal within the grid does not change

#### Scenario: Ctrl+` blurs the focused terminal
- **WHEN** a terminal has input focus and the user presses `Ctrl+\``
- **THEN** keyboard focus leaves the terminal and returns to chrome-navigation mode, and `Ctrl+\`` is not sent to the terminal's pty as input

### Requirement: New-session affordance per workspace
The UI SHALL provide a per-workspace control to spawn a new session in that workspace, which calls the session-spawn API (resolving the directory via the workspace registry) and adds the resulting session to the grid.

The new-session control SHALL include a worktree toggle. When enabled, the spawn call SHALL include `worktree: true` per the `worktree-sessions` capability. The toggle's last-used state SHALL be remembered per workspace (e.g. via `localStorage`, keyed by workspace) and SHALL be used to pre-set the toggle the next time the new-session control is opened for that same workspace.

#### Scenario: Spawning a session from the rail
- **WHEN** the user activates the new-session control for workspace `kowboy` and supplies a label
- **THEN** the daemon spawns a session `garage/kowboy/<label>` via the registry-resolved directory, and it appears in the rail and (if `kowboy` is focused) in the terminal grid

#### Scenario: Worktree toggle state is remembered per workspace
- **WHEN** the user enables the worktree toggle while spawning a session for workspace `kowboy`, and later reopens the new-session control for `kowboy`
- **THEN** the worktree toggle is pre-set to enabled; opening the new-session control for a different workspace `garage-dev` that has no remembered state does not inherit `kowboy`'s toggle state


### Requirement: Help overlay
The UI SHALL support a `?` keybinding that opens a help overlay listing all keybindings (`1`–`9`, `[`, `]`, `a`, `Ctrl+\``, `?`, `Esc`). `Esc` SHALL close the overlay when it is open. The `?` keybinding SHALL be suppressed while a terminal has keyboard focus, consistent with other global keybindings, so typing `?` into a terminal does not open the overlay.

#### Scenario: ? opens the help overlay
- **WHEN** chrome-navigation mode has focus (no terminal focused) and the user presses `?`
- **THEN** a help overlay appears listing all keybindings and their effects

#### Scenario: Esc closes the help overlay
- **WHEN** the help overlay is open and the user presses `Esc`
- **THEN** the overlay closes and the previous view (rail + grid) is visible again

#### Scenario: ? typed into a terminal does not open the overlay
- **WHEN** a terminal has input focus and the user types `?` as part of program input
- **THEN** the `?` is sent to the terminal's pty and no help overlay appears

### Requirement: Restorable sessions in the rail
The workspace rail SHALL render restorable sessions (status `restorable`) visually distinct from live sessions — dimmed, with a `⟳` glyph — and SHALL provide a restore control for each. When every session belonging to a workspace's deck is restorable, the rail SHALL additionally show a restore-all control for that workspace (e.g. the post-reboot state where tmux came back empty). Activating a restore control SHALL call the restore endpoint and, on success, cause the restored session to appear live in the grid.

#### Scenario: Restorable session renders dimmed with the ⟳ glyph
- **WHEN** `GET /api/sessions` reports `garage/kowboy/checkout` with status `restorable`
- **THEN** the rail renders that session dimmed with a `⟳` glyph instead of the usual status glyph, alongside a restore control

#### Scenario: Restore-all control appears when a workspace's entire deck is restorable
- **WHEN** all sessions belonging to workspace `kowboy` are reported `restorable` (e.g. immediately after a reboot, before anything is restored)
- **THEN** the rail shows a restore-all control for `kowboy` in addition to each session's individual restore control

#### Scenario: Restoring from the rail puts the session back in the grid live
- **WHEN** the user activates the restore control for a restorable session `garage/kowboy/checkout` while `kowboy` is the focused workspace
- **THEN** the daemon restores the session, and once restore succeeds the grid renders `garage/kowboy/checkout` as a live, interactive terminal (no longer dimmed/restorable in the rail)

### Requirement: Worktree finish prompt
For a session that was spawned with a worktree (per the `worktree-sessions` capability), the close flow's second step (after the two-step ✕ kill confirm) SHALL offer an inline `merge` / `discard` / `keep` choice instead of simply completing. Each action SHALL be reachable with a single click (no modal). Any error returned by the corresponding `POST /api/worktrees/finish` call (e.g. a merge conflict) SHALL be surfaced inline in that same prompt, and the prompt SHALL remain available for the user to retry a different action. Sessions that were not spawned with a worktree SHALL keep the existing two-step ✕ confirm behavior unchanged, with no finish prompt shown.

#### Scenario: Finish prompt appears after killing a worktree session
- **WHEN** the user completes the two-step ✕ confirm for a worktree session `garage/kowboy/feature`
- **THEN** the session is killed and an inline prompt appears offering `merge`, `discard`, and `keep` for that session's worktree

#### Scenario: Merge conflict surfaces inline and keeps the worktree
- **WHEN** the user selects `merge` in the finish prompt and the daemon responds 409 with git's conflict message
- **THEN** the prompt displays git's error message inline, and the worktree and branch remain on disk (nothing is silently discarded)

#### Scenario: Non-worktree sessions keep the existing close behavior
- **WHEN** the user completes the two-step ✕ confirm for a session that was not spawned with `worktree:true`
- **THEN** the session is killed and no finish prompt is shown, matching existing (pre-p5) behavior
