## ADDED Requirements

### Requirement: Changes pane for the focused workspace
The UI SHALL render a changes pane beside the terminal grid, scoped to the currently focused workspace, showing that workspace's changed-file list (with per-file +/− stats) and a scrollable unified diff, sourced from `GET /api/diff/:workspace`.

#### Scenario: Pane shows focused workspace's changes
- **WHEN** workspace `kowboy` is focused and has two changed files
- **THEN** the changes pane lists both files with their +/− stats and renders a scrollable unified diff for them

#### Scenario: Pane updates when focus moves to a different workspace
- **WHEN** the user switches the focused workspace from `kowboy` to `garage-dev`
- **THEN** the changes pane now shows `garage-dev`'s changed files and diff, not `kowboy`'s

### Requirement: Tab toggles file-list/diff emphasis
Pressing `Tab` while the changes pane is active SHALL toggle which of the two pane regions (file list, diff) is visually emphasized (e.g. expanded/focused for keyboard interaction), without leaving the pane.

#### Scenario: Tab switches emphasis from file list to diff
- **WHEN** the changes pane's file list is emphasized and the user presses `Tab`
- **THEN** the diff region becomes emphasized and the file list de-emphasizes

### Requirement: j/k step through changed files in the pane
While the changes pane is active, `j` and `k` SHALL move the selected file in the changed-file list to the next/previous entry respectively, updating the diff region to show the newly selected file.

#### Scenario: k moves to the previous file
- **WHEN** the changes pane has three changed files with the second selected and the user presses `k`
- **THEN** the first file becomes selected and the diff region scrolls/updates to show it

#### Scenario: j moves to the next file
- **WHEN** the changes pane has three changed files with the first selected and the user presses `j`
- **THEN** the second file becomes selected and the diff region updates to show it

### Requirement: Diff freshness — refetch on session done and manual refresh
The changes pane SHALL refetch the focused workspace's diff when any session belonging to that workspace transitions to status `done` (observed via the existing `/api/events` SSE channel), and SHALL also provide a manual refresh control that refetches on demand.

#### Scenario: Pane refetches when a session in the focused workspace finishes
- **WHEN** the focused workspace is `kowboy` and a session within `kowboy` transitions from `working` to `done` via `/api/events`
- **THEN** the changes pane issues a fresh `GET /api/diff/kowboy` call and updates its contents

#### Scenario: Done transition in a non-focused workspace does not trigger refetch
- **WHEN** the focused workspace is `garage-dev` and a session in workspace `kowboy` (not focused) transitions to `done`
- **THEN** the changes pane does not refetch (it is not showing `kowboy`)

#### Scenario: Manual refresh control refetches on demand
- **WHEN** the user activates the changes pane's manual refresh control
- **THEN** the daemon is queried again via `GET /api/diff/:workspace` for the focused workspace and the pane updates with the result

### Requirement: Review mode entry, layout, and exit
Pressing `r` SHALL enter a full-screen review mode for the focused workspace, overlaying the rest of the pit wall, showing a file rail (with per-file viewed checkmarks) and a continuous full-width diff across all changed files. Pressing `Esc` while in review mode SHALL exit back to the normal pit-wall layout.

#### Scenario: r enters full-screen review mode
- **WHEN** the focused workspace has changed files and the user presses `r`
- **THEN** the UI shows a full-screen overlay with a file rail and a continuous diff, and the normal grid/pane layout is no longer visible

#### Scenario: Esc exits review mode
- **WHEN** the user is in review mode and presses `Esc`
- **THEN** the UI returns to the normal pit-wall layout (rail, grid, changes pane)

### Requirement: j/k navigate files within review mode
While in review mode, `j` and `k` SHALL move between files in the file rail, scrolling the continuous diff to the selected file's section.

#### Scenario: j advances to the next file in review mode
- **WHEN** review mode is open with the first of four files selected and the user presses `j`
- **THEN** the second file becomes selected in the rail and the continuous diff scrolls to that file's section

### Requirement: v marks a file viewed and auto-advances
While in review mode, pressing `v` SHALL mark the currently selected file as viewed (shown as a checkmark in the file rail) and SHALL automatically advance selection to the next file in the rail that is not yet marked viewed.

#### Scenario: v checks off the current file and jumps to the next unviewed one
- **WHEN** review mode has files A (unviewed, selected), B (viewed), C (unviewed) in rail order, and the user presses `v`
- **THEN** A is marked viewed with a checkmark, and selection automatically advances to C (the next unviewed file), skipping already-viewed B

#### Scenario: Marking the last unviewed file leaves selection in place
- **WHEN** review mode has only one unviewed file remaining and the user presses `v` on it
- **THEN** it is marked viewed and selection does not move (there is no next unviewed file to advance to)

### Requirement: Viewed state persists across reloads and resets on content change
Per-file viewed state SHALL be scoped per workspace and persist across UI reloads (e.g. a browser refresh). For a given file, if that file's diff content changes since it was last marked viewed, the file's viewed state SHALL reset to unviewed.

#### Scenario: Viewed checkmarks survive a page reload
- **WHEN** a file is marked viewed in review mode and the browser page is reloaded
- **THEN** reopening review mode for the same workspace shows that file still marked as viewed

#### Scenario: Changed diff content resets viewed state for that file
- **WHEN** a file was marked viewed, and a subsequent diff refetch shows different diff content for that same file (e.g. the session produced a new edit)
- **THEN** that file's viewed state resets to unviewed, while other unchanged files keep their viewed state

### Requirement: Review and pane keybindings obey terminal-focus suppression
`Tab`, `j`, `k`, `r`, `Esc`, and `v` SHALL follow the same suppression rule as the P1 pit-wall keybindings: they SHALL NOT fire while keyboard focus is inside a terminal's input, and SHALL be delivered to the terminal's pty instead in that state.

#### Scenario: v typed into a terminal does not mark a file viewed
- **WHEN** a terminal has input focus and the user types `v` as part of a prompt to Claude
- **THEN** `v` is sent to that terminal's pty as input, and no file's viewed state changes

#### Scenario: r typed into a terminal does not open review mode
- **WHEN** a terminal has input focus and the user types `r` as part of program input
- **THEN** `r` is sent to the pty and review mode does not open
