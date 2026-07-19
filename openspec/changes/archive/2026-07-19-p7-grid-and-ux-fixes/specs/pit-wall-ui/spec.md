# pit-wall-ui (delta)

## MODIFIED Requirements

### Requirement: Worktree finish prompt
For a session that was spawned with a worktree (per the `worktree-sessions` capability), the close flow's second step (after the two-step ✕ kill confirm) SHALL offer an inline `merge` / `discard` / `keep` choice instead of simply completing. `merge` and `keep` SHALL each be reachable with a single click (no modal). `discard` SHALL require a two-step armed confirmation in the same style as the ✕ kill confirm: the first activation arms the control (visually marked as destructive, e.g. "discard branch?") for a bounded window (~3s), and only a second activation within that window performs the discard; the window elapsing disarms it. Any error returned by the corresponding `POST /api/worktrees/finish` call (e.g. a merge conflict) SHALL be surfaced inline in that same prompt, and the prompt SHALL remain available for the user to retry a different action. Sessions that were not spawned with a worktree SHALL keep the existing two-step ✕ confirm behavior unchanged, with no finish prompt shown.

#### Scenario: Finish prompt appears after killing a worktree session
- **WHEN** the user completes the two-step ✕ confirm for a worktree session `garage/kowboy/feature`
- **THEN** the session is killed and an inline prompt appears offering `merge`, `discard`, and `keep` for that session's worktree

#### Scenario: Discard requires a second, armed click
- **WHEN** the user clicks `discard` once in the finish prompt
- **THEN** no git operation runs; the control arms into a destructive-styled confirmation, and only a second click within the arm window calls `POST /api/worktrees/finish` with `action:"discard"`

#### Scenario: Armed discard disarms after the window
- **WHEN** the user clicks `discard` once and then does nothing for the arm window's duration
- **THEN** the control returns to its unarmed state and a later single click does not discard

#### Scenario: Merge conflict surfaces inline and keeps the worktree
- **WHEN** the user selects `merge` in the finish prompt and the daemon responds 409 with git's conflict message
- **THEN** the prompt displays git's error message inline, and the worktree and branch remain on disk (nothing is silently discarded)

#### Scenario: Non-worktree sessions keep the existing close behavior
- **WHEN** the user completes the two-step ✕ confirm for a session that was not spawned with `worktree:true`
- **THEN** the session is killed and no finish prompt is shown, matching existing (pre-p5) behavior

### Requirement: Help overlay
The UI SHALL support a `?` keybinding that opens a help overlay listing all keybindings (`1`–`9`, `[`, `]`, `a`, `\`, `m`, `Tab`, `j`/`k`, `r`, `v`, `o`, `Ctrl+\``, `?`, `Esc`). The overlay SHALL additionally include a status-legend section explaining every session status glyph and its meaning: `●` needs-input (Claude is waiting on you), `◐` working, `✓` done, `○` idle, `⟳` restorable. `Esc` SHALL close the overlay when it is open. The `?` keybinding SHALL be suppressed while a terminal has keyboard focus, consistent with other global keybindings, so typing `?` into a terminal does not open the overlay.

#### Scenario: ? opens the help overlay
- **WHEN** chrome-navigation mode has focus (no terminal focused) and the user presses `?`
- **THEN** a help overlay appears listing all keybindings and their effects

#### Scenario: Overlay explains the status glyphs
- **WHEN** the help overlay is open
- **THEN** a legend section lists each of `●` `◐` `✓` `○` `⟳` with its status name and a one-line meaning

#### Scenario: Esc closes the help overlay
- **WHEN** the help overlay is open and the user presses `Esc`
- **THEN** the overlay closes and the previous view (rail + grid) is visible again

#### Scenario: ? typed into a terminal does not open the overlay
- **WHEN** a terminal has input focus and the user types `?` as part of program input
- **THEN** the `?` is sent to the terminal's pty and no help overlay appears

## ADDED Requirements

### Requirement: First-run empty state
When no workspaces are registered, the grid area SHALL render an onboarding empty state instead of a bare placeholder line: a one-line description of what the product does, a short numbered getting-started list (add a workspace → spawn sessions → triage on `●`/`a`), a primary call-to-action that opens the add-workspace flow, and a hint that `?` shows the keybindings. When a workspace exists but has no sessions, the grid's empty message SHALL point at the workspace's new-session control.

#### Scenario: Empty install shows onboarding with a working CTA
- **WHEN** the UI loads with zero registered workspaces
- **THEN** the grid shows the onboarding card, and activating its call-to-action opens the same add-workspace flow as the header control

#### Scenario: Empty workspace points at the spawn control
- **WHEN** the focused workspace has no sessions
- **THEN** the grid's empty message tells the user where the new-session control is, rather than only stating that no sessions exist

### Requirement: Inline session-creation errors and worktree label
The rail's new-session control SHALL render creation failures (duplicate label 409, unknown workspace 404, non-git-repo 400, and transport errors) as readable inline text adjacent to the form — not solely as a glyph whose message requires hover. The worktree toggle SHALL be labeled with the word "worktree" (not an abbreviation), with the isolation consequence available in supporting text or tooltip.

#### Scenario: Duplicate label failure is readable without hovering
- **WHEN** the user submits a new-session label that already exists in that workspace
- **THEN** the error text (naming the label and the workspace) is visible inline near the form without any hover interaction

#### Scenario: Worktree toggle is self-describing
- **WHEN** the new-session form is open
- **THEN** the isolation toggle's visible label reads "worktree"

### Requirement: Chrome affordance standards
Interactive chrome SHALL meet these standards: all buttons render `cursor: pointer`; the workspace-removal control is labeled distinctly from session kill (e.g. `unreg` with an explanatory tooltip) so the same `✕` glyph never means both a safe and a destructive action; per-row control clusters (rename / open / unregister / add) have enough padding that adjacent targets are not sub-16px; and the `faint`/`dim` text tokens are raised (faint ≥ `#55627a`, dim ≥ `#7d8ba1` against `#0b0e14`) so 10–11px annotation text remains legible.

#### Scenario: Workspace removal is visually distinct from session kill
- **WHEN** the user hovers a workspace header row and a session cell's title bar
- **THEN** the workspace-removal control does not render as a bare `✕` (session kill does), and its tooltip states that sessions keep running

#### Scenario: Buttons signal interactivity
- **WHEN** the user hovers any button in the rail, grid header, cell title bars, or changes pane
- **THEN** the cursor renders as a pointer

### Requirement: Changes pane auto-collapse on narrow viewports
Below a viewport width threshold (~1080px), the changes pane SHALL automatically collapse to its 32px toggle strip so the terminal grid keeps usable width; expanding it manually SHALL still be possible, and the pane SHALL return to its prior state when the viewport widens past the threshold.

#### Scenario: Half-screen window folds the pane
- **WHEN** the window is resized below the threshold with the changes pane expanded
- **THEN** the pane collapses to the toggle strip and the grid retains the reclaimed width

### Requirement: Visible review-mode entry
The changes pane header SHALL provide a visible control that enters full-screen review mode, equivalent to the `r` keybinding, so review mode is discoverable by mouse and not reachable only through a keybinding. The control SHALL trigger the same unconditional diff refetch as the `r` binding.

#### Scenario: Review button opens review mode
- **WHEN** the user clicks the review control in the changes pane header
- **THEN** full-screen review mode opens for the focused workspace, identically to pressing `r` (including the diff refetch on entry)

### Requirement: Editor-open fallback to workspace root
When the `o` keybinding fires with no diff file selected, it SHALL open the focused workspace's root in the editor (matching the documented "open selected file (or workspace root)" behavior) instead of doing nothing.

#### Scenario: o with no selection opens the root
- **WHEN** the focused workspace's diff list is empty (no selected file) and the user presses `o`
- **THEN** the workspace root opens in the editor via the existing open-editor endpoint
