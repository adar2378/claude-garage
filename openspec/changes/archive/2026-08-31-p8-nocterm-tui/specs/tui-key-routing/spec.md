# tui-key-routing

## ADDED Requirements

### Requirement: Three key layers with a visible target chip
Keyboard input SHALL be governed by exactly one active layer at a time: `garage` (default; single-key app commands), `engaged` (all input flows to one tile's PTY), or `overlay` (an open overlay captures keys). The bottom strip SHALL always display where keys currently go (e.g. `keys → garage`, `keys → apexlabs/api-fix`). Overlays SHALL never stack more than one deep.

#### Scenario: Chip tracks the active layer
- **WHEN** the user engages a tile
- **THEN** the strip chip changes from `keys → garage` to `keys → <workspace>/<label>` in the same frame as the layer switch

#### Scenario: Garage typing never reaches an agent
- **WHEN** the layer is `garage` and the user types letters
- **THEN** no bytes are written to any tile PTY

### Requirement: Engage and disengage
Pressing Enter (or clicking a tile) in the garage layer SHALL engage the focused tile. While engaged, every key event except the reserved disengage chord SHALL be forwarded to that tile's PTY. The disengage chord SHALL be a single Ctrl-chord (Ctrl+Q unless the vendored framework's Ctrl+G debug intercept has been disabled, in which case Ctrl+G per the UX mockup), SHALL return to the garage layer, and SHALL NOT be forwarded. Esc SHALL always be forwarded while engaged (never used as a layer control).

#### Scenario: Esc reaches Claude Code
- **WHEN** the user presses Esc while engaged on a Claude Code session
- **THEN** the byte `0x1b` is written to the tile PTY and the layer remains `engaged`

#### Scenario: Disengage chord returns to garage
- **WHEN** the user presses the disengage chord while engaged
- **THEN** the layer becomes `garage`, the chip updates, and the chord is not written to the PTY

### Requirement: Verbatim byte re-encoding
While engaged, the TUI SHALL re-encode parsed key events back into the raw byte sequences a real terminal would send, covering at minimum: printable characters; Ctrl+letter (0x01–0x1a); Alt+character (ESC prefix); arrows, Home, End with any modifier combination (`CSI 1;<mod><final>`); Enter, Tab, Shift+Tab (`CSI Z`), Backspace, Delete, Insert, PageUp/PageDown with modifiers. The framework's built-in key translation SHALL NOT be used (it drops modifiers).

#### Scenario: Alt+arrow word jump end-to-end
- **WHEN** the user presses Alt+Right while engaged on a Claude Code composer containing text
- **THEN** the PTY receives `ESC [1;3C` and the composer cursor moves by one word

#### Scenario: Control characters pass
- **WHEN** the user presses Ctrl+C while engaged
- **THEN** the PTY receives the single byte `0x03`

#### Scenario: Shift+Tab passes
- **WHEN** the user presses Shift+Tab while engaged
- **THEN** the PTY receives `ESC [Z` (Claude Code mode cycling works)

### Requirement: Paste forwarding
Text pastes (bracketed paste from the host terminal, and the framework's synthetic paste events for coalesced input) SHALL be forwarded to the engaged tile's PTY wrapped in bracketed-paste guards (`ESC [200~ … ESC [201~`). Coalesced ordinary keystrokes SHALL NOT be converted into pastes: the vendored framework patch that keeps short character runs as individual key events SHALL be maintained (Enter is never treated as a printable batch character).

#### Scenario: Real paste arrives bracketed
- **WHEN** the user pastes a multi-line snippet while engaged on Claude Code
- **THEN** the composer receives it as one bracketed paste (no line is submitted by the embedded newlines)

#### Scenario: Fast typing stays keystrokes
- **WHEN** three keystrokes including Enter arrive in a single stdin read
- **THEN** each is delivered as its own key event and Enter behaves as Enter

### Requirement: Garage-layer bindings
In the garage layer: `1`–`9` SHALL switch the focused workspace, `[`/`]` SHALL cycle the focused tile, `Enter` SHALL engage, `a` SHALL jump per tui-triage, `A` SHALL open the triage queue, `n`/`N` SHALL spawn a session / worktree session in the focused workspace via `POST /api/sessions`, `?` SHALL toggle help, and `q` SHALL quit the TUI (leaving tmux sessions running).

#### Scenario: Spawn from the rail
- **WHEN** the user presses `n` with workspace `garage` focused
- **THEN** `POST /api/sessions` is called for workspace `garage` with a generated label and the new tile appears after the sessions refetch

### Requirement: p8.1 session lifecycle bindings
In the garage layer: `m` SHALL toggle maximize per tui-wall; `Enter` (or a click) on a focused RESTORABLE tile SHALL restore it via `POST /api/sessions/restore {id}` with an optimistic "restoring…" placeholder, a refetch on response, and failures surfaced in the strip notice (engage itself SHALL still require a live session); `R` SHALL restore ALL restorable sessions in the focused workspace via parallel per-id restore calls (one failure never blocks the rest — the web UI's restore-all shape); `x` SHALL close the focused session with an armed double-press (the second `x` on the same session within 3 seconds confirms; the strip shows "press x again to close <label>"; any other key disarms) — a live session via `DELETE /api/sessions/<id>`, a restorable one via `DELETE /api/sessions/<id>?meta=1` (dropping only the stored resume metadata; the plain DELETE returns 404 with no live tmux session). When the delete response carries a worktree record, the worktree SHALL be kept (v1) and the strip SHALL say so ("worktree kept: <branch> — merge or discard in the web wall"). `w` SHALL open an add-workspace overlay with a text field for a directory path: `~` is expanded, the directory's existence is validated client-side, the name derives web-UI-style (basename slug, `-2` suffix on collision), registration is `PUT /api/workspaces` followed by a refetch and focusing the new workspace; Esc cancels. The help overlay SHALL list all of these keys.

#### Scenario: Enter restores a restorable tile
- **WHEN** the focused tile is a restorable placeholder and the user presses Enter (or clicks it)
- **THEN** the placeholder shows "restoring…", `POST /api/sessions/restore {id}` fires, and after the refetch the tile is live (a failure shows its reason in the strip instead)

#### Scenario: R restores the whole workspace
- **WHEN** the focused workspace holds two restorable sessions and one live one
- **THEN** `R` fires one restore call per restorable id in parallel and the live session is untouched

#### Scenario: x-x closes with arming
- **WHEN** the user presses `x` once on a focused live session
- **THEN** nothing is deleted and the strip shows "press x again to close <label>"; a second `x` within 3s calls `DELETE /api/sessions/<id>`; any other key first disarms

#### Scenario: Closing a restorable session drops only its metadata
- **WHEN** the user confirms `x`-`x` on a restorable placeholder
- **THEN** the TUI calls `DELETE /api/sessions/<id>?meta=1` and the placeholder disappears after the refetch (web `DELETE` behavior is unchanged)

#### Scenario: Worktree kept on close
- **WHEN** a closed session's delete response carries a worktree record
- **THEN** the worktree is not removed and the strip notice names the branch and points at the web wall

#### Scenario: w adds a workspace
- **WHEN** the user presses `w`, types `~/dev/proj` and presses Enter (the directory exists)
- **THEN** the path expands, `PUT /api/workspaces {name: "proj", dir}` fires, the new workspace is focused after the refetch; a nonexistent directory keeps the overlay open with an inline error and Esc cancels

### Requirement: p8.3 workspace removal and rail focus marker
In the garage layer, `X` (capital) SHALL remove the FOCUSED workspace's registration with an armed double-press: the first `X` arms and the strip shows "press X again to remove <name> (sessions keep running)" (the p8.4 requirement appends a `K` clause when the workspace has live sessions); a second `X` on the same workspace within 3 seconds confirms via registry-only `DELETE /api/workspaces/<name>` (NEVER the `?sessions=kill` variant — tmux is untouched); any other key disarms. The `x` close arm and the `X` remove arm SHALL be independent instances of the same armed-action machine (pressing one key disarms the other's pending arm — "any other key disarms" applies across both). After the post-delete refetch, the workspace's live sessions SHALL reappear as a synthesized unregistered group (registered: false) — never become invisible. Pressing `X` while an unregistered group is focused SHALL NOT call the API: the strip SHALL explain instead ("already unregistered — sessions live in tmux; x closes them individually"). The help overlay SHALL list `X`.

The rail SHALL mark WHICH SESSION is focused, not just the workspace: the focused session's row carries a `▸` marker prefix and a bright/bold (never amber) emphasis, tracking every focus change (digits, `[`/`]`, rail and tile clicks, the `a` jump, the triage-queue jump, spawn, restore). The marker column SHALL be reserved on every session row so rows never shift horizontally as focus moves, and the rail SHALL keep its one-line-per-row layout so the click mapping is unchanged.

#### Scenario: X-X removes the focused workspace registry-only
- **WHEN** the user presses `X` twice within 3s with registered workspace `proj` focused (two live sessions)
- **THEN** the first `X` deletes nothing and the strip shows "press X again to remove proj (sessions keep running)"; the second calls `DELETE /api/workspaces/proj` with no query parameters, both tmux sessions stay alive, and after the refetch they reappear under a synthesized `proj` group with registered: false

#### Scenario: Any other key disarms a pending removal
- **WHEN** the user presses `X`, then any other key, then `X` again
- **THEN** the intermediate key disarms the pending removal and the final `X` re-arms (shows the notice) instead of removing

#### Scenario: X on an unregistered group explains instead of removing
- **WHEN** the focused group is a synthesized unregistered workspace and the user presses `X`
- **THEN** no API call is made and the strip shows "already unregistered — sessions live in tmux; x closes them individually"

#### Scenario: Rail marker follows focus
- **WHEN** the user cycles focus with `]` (or clicks another session's rail row)
- **THEN** the `▸` marker and the bright/bold emphasis move to the newly focused session's rail row in the next frame, and no rail row shifts horizontally

### Requirement: p8.4 kill-all removal confirm
While a p8.3 `X` remove arm is active on a registered workspace, `K` SHALL confirm the removal WITH session kill via `DELETE /api/workspaces/<name>?sessions=kill` (the daemon kills every live `garage/<name>/*` tmux session, then unregisters; worktrees stay untouched per the daemon contract). When the armed workspace has `n > 0` live sessions the arm notice SHALL read "press X again to remove <name> (sessions keep running) · K to also kill its <n> sessions"; with zero live sessions the `K` clause SHALL be omitted (the p8.3 wording unchanged). After a `K` confirm the strip SHALL report "removed <name> · killed <n> sessions" from the response's `killedSessions`, and any `failedSessions` SHALL be named instead of silently dropped. `X`-`X` SHALL remain registry-only (never `?sessions=kill`). Outside an active (unexpired) `X` arm, `K` SHALL stay an ordinary unbound garage key (consumed; typing hint applies) and SHALL never arm anything. Cross-disarm rules are unchanged: any key other than `X`/`K` disarms the pending removal, and `x` still cross-disarms it. Unregistered groups cannot arm (p8.3), so `K` can never kill-remove them. The help overlay SHALL list the `X` then `K` confirm.

#### Scenario: X then K removes the workspace and kills its sessions
- **WHEN** registered workspace `proj` has two live sessions, the user presses `X` (strip: "press X again to remove proj (sessions keep running) · K to also kill its 2 sessions"), then `K` within 3s
- **THEN** `DELETE /api/workspaces/proj?sessions=kill` fires, both tmux sessions are killed, the registration is gone, and the strip shows "removed proj · killed 2 sessions"

#### Scenario: Zero live sessions omit the K clause
- **WHEN** the user presses `X` on a registered workspace with no live sessions
- **THEN** the arm notice is exactly "press X again to remove <name> (sessions keep running)" with no `K` clause

#### Scenario: K outside an arm stays unbound
- **WHEN** the user presses `K` in the garage layer with no removal armed (or after the 3s window expired)
- **THEN** no API call is made, nothing arms, and the key is consumed like any unbound garage key (typing hint)

#### Scenario: X-X still leaves sessions running
- **WHEN** the user confirms with `X`-`X` instead of `K`
- **THEN** the DELETE carries no query parameters and every live session keeps running (p8.3 behavior unchanged)
