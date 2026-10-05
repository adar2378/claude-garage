# Changelog

## 0.6.0 — folder groups in the rail

### Added

- **Sibling workspaces sit together.** Workspaces that live in the same
  parent folder are now adjacent in the rail, with a faint label row
  naming the folder (for example `elite-traders`) above the first one.
  Folder case is ignored. Workspaces with no sibling get no label.
- **Blocked work floats as a family.** If any workspace in a folder has a
  session waiting on you, the whole folder moves up, and the blocked
  workspace comes first inside it.

Workspace numbers `1`-`9` are unchanged. Label rows have no number and
clicking one does nothing.

## 0.5.0 — terminal only

### Breaking

- **The web wall is gone.** garage is now a terminal app. `npx
  claude-garage` with no subcommand opens the TUI; `tui` still works as
  an alias. The daemon no longer serves a page on `http://127.0.0.1:4747`
  or terminal WebSockets on `/term`. Its HTTP API is unchanged.
- **Diff review is unavailable** until it lands in the TUI (on the
  roadmap). The daemon's diff API stays in place for it.
- Clicking a macOS notification no longer opens a browser tab.

### Added

- **Finish a worktree from the TUI.** Closing a worktree session (`x x`,
  live or restorable) opens an overlay: `m` merges the branch into the
  branch checked out in the repo (shown in the prompt), `d d` discards
  it, `k` or `Esc` keeps it. A dirty worktree or a merge conflict shows
  inline, and the overlay stays open so you can retry or keep.
- **`I` installs hooks too.** One key now installs the Claude Code hooks
  (instant status) and the statusline (context meter), with one notice
  that covers both, including "already installed".

### Fixed

- **A failed merge is aborted.** When finishing a worktree hits a
  conflict, the daemon runs `git merge --abort`, so the repo is never
  left mid-merge.

### Removed

- The React web wall (`ui/`), the `/term` WebSocket bridge, static file
  serving and the Vite dev origins.
- Dependencies `@fastify/static`, `node-pty`, `ws` and `concurrently`,
  plus the `dev`, `postinstall` and `prepack` scripts.

## 0.4.1 — no more typing paths

### Added

- **Paste a path into the TUI's add-workspace field.** In the `w`
  overlay, a paste now lands in the field instead of being dropped.
  Dragging a folder from Finder into Ghostty works too: the text is
  cleaned of shell escaping (`My\ Proj` → `My Proj`), wrapping quotes
  and extra lines. Pastes anywhere else outside a tile are still
  swallowed, so pasted text can never fire wall commands.
- **`Ctrl+O` opens the native folder picker** from the `w` overlay,
  reusing the daemon's `POST /api/pick-directory` (macOS). The picked
  folder fills the field; cancel keeps what you typed; a non-macOS
  daemon shows an inline "type or paste a path" hint. The footer now
  reads `Enter add · ^O browse · Esc cancel`.

## 0.4.0 — restart

### Added

- **`claude-garage restart` / `--sessions` / `--all`.** Restarts the
  daemon in place (health pid, lsof fallback, wait for health, print the
  version); `--sessions` additionally respawns every idle/done session's
  tmux pane with `claude --resume`, printing one line per restarted,
  skipped and failed session; `--all` includes busy sessions.
- **`POST /api/daemon/restart`.** Self-replace: spawns a detached
  successor daemon with the same environment, replies 202 with its pid,
  then closes and exits; the successor waits for the old pid's port to
  free before listening.
- **`POST /api/sessions/restart`.** `{id}` or `{all: true, force?}`
  respawns each target's tmux pane with `claude --resume <id>` in place,
  skipping `working`/`needs-input` sessions unless `force`. Returns
  `{restarted: [{id, resumed}], skipped: [{id, status}], failed: [{id,
  error}]}`.
- **TUI chords.** `r r` restarts the focused session (armed double-press,
  warns and forces on a busy session), `r a` restarts every idle/done
  session in the focused workspace, `r d` restarts the daemon; all three
  show a strip notice and are listed in `?`.
- **Web wall control.** A restart button (`↻`) in the session cell's
  hover controls, with the same armed double-click as close.

## 0.3.4

### Added

- **The pit pet comes to the TUI.** Arthur, Papito and Segan now live in
  the bottom strip as one-row sprites with the same derived moods as the
  web wall (sleeping, watching, alert-with-`!`, boxed when the daemon
  drops, celebrating when the last blocked session clears) and the same
  manners (the cat ignores you, the duck never blinks, the pup does
  zoomies). `P` cycles the roster; the choice persists in `wall.json`;
  clicking an alert pet is the `a` jump, clicking it otherwise pets it.
- **Pets talk.** Every few minutes, in character, the strip shows a line
  from the pet: hydrate, stretch, encouragement, a proud word when you
  clear the queue, a late-night nudge, a heads-up when a usage window
  runs hot, and a greeting when the daemon comes back. Chatter yields to
  anything that matters: never while a session needs input, never over a
  real notice, never inside a tile. Off with the pet; no separate toggle.

### Fixed

- **Option+Enter works inside garage sessions again.** 0.3.3's tmux
  extended-keys config made tmux re-encode Option+Enter as
  `ESC[27;3;13~`, which Claude Code doesn't parse. The daemon now also
  sets `extended-keys-format csi-u`, so modified Enter reaches Claude
  Code in the kitty form it already understands (tmux ≥ 3.5). Applies to
  the TUI, the web wall, and plain `tmux attach` alike.
- **Terminal.app note.** macOS Terminal speaks neither the kitty keyboard
  protocol nor modifyOtherKeys, so Shift+Enter can't reach Claude Code
  through the wall there. Use Ghostty, iTerm2, kitty or WezTerm, or add a
  Terminal.app key mapping for Shift+Return that sends `\033[13;2u`.

## 0.3.3

### Fixed

- **Claude Code's own keybindings now work in the TUI.** Shift+Enter
  (insert newline) reaches Claude Code instead of submitting: the wall
  speaks the kitty keyboard protocol to your terminal (when it supports
  it), encodes modified Enter as CSI-u (`ESC[13;2u`), and the daemon
  applies Claude Code's documented tmux config (`extended-keys on` +
  `terminal-features 'xterm*:extkeys'`) idempotently at boot and on every
  session spawn — which also fixes Shift+Enter for plain `tmux attach`
  users of garage sessions. Ctrl+Enter passes through too; Option+Enter,
  `\`+Enter, and Ctrl+J always worked.
- **Clicking a URL in a tile opens it.** The wall's mouse capture meant
  your terminal's own Cmd+click linkifier never saw clicks; now a left
  click on an `http(s)://` URL in any tile's text (live or frozen) opens
  it via macOS `open` with a notice, without engaging the tile.

## 0.3.2

### Changed

- **The title is the name now (TUI).** `claude-1` means nothing to a user;
  when Claude Code broadcasts what a session is about ("George employment
  history"), that title renders as the session's primary name in the tile
  bar, workspace rail, and triage queue — with the auto-label demoted to a
  dim trailing id in the tile bar. No title → the label renders exactly as
  before.

### Fixed

- **Context meters no longer overstate usage ~5× on current models.** The
  transcript fallback assumed a 200k context window unless the model id
  carried the old `[1m]` beta marker — but the Claude 5 family and the
  4.6+ generation are natively 1M and their ids carry no marker, so a
  session at ~5% showed ~27%. The default window is now 1M, with a
  known-200k list (Haiku, the 3.x family, pre-4.6 Opus/Sonnet). Better
  still: statusline posts now teach the daemon each model's *actual*
  window from Claude Code's own `context_window.context_window_size`
  payload field, which wins over every heuristic. (The statusline-fed
  percentage — the `I` install — was always correct; only the fallback
  was wrong.)
- **`npm run build:tui` no longer produces a binary macOS refuses to run.**
  Copying over the dist binary's existing inode while a running TUI still
  had it mapped made arm64 macOS SIGKILL every fresh exec of the file; the
  script now removes the old binary before copying.

## 0.3.0 — the TUI arc

A full-screen terminal wall joins the browser wall as a second, first-class
surface — same daemon, same tmux sessions, no lock-in either way.

### Added

- **`claude-garage tui`**: a full-screen terminal wall (Rust/ratatui, pinned
  published crates — no vendored forks). Workspace rail, a live 6-tile grid
  with overflow rail, three-layer key routing (garage / engaged / overlay)
  with verbatim byte-exact key passthrough (Alt+arrows, paste, everything —
  the bug the browser terminal has always had), needs-input triage (`a`
  jump, `A` queue overlay), frozen local scrollback, and a `?` help overlay.
  Ships as a prebuilt `arm64` binary on macOS; other platforms self-build
  once via `cargo` on first run.
- **Views (groups)**: partition a workspace's sessions into named views —
  `d` detaches the focused session into its own view or rejoins it to the
  default, `D` moves it to a chosen group, `Tab` cycles views. A view strip
  appears once a workspace has 2+ views; the 6-tile cap applies per view;
  blocked sessions in background views still light the rail/badge and are
  still reachable by `a`/queue jumps. Assignments persist locally
  (`~/.garage/wall.json`).
- **Auto-subtitles**: Claude Code's own terminal-title updates (tmux's
  `#{pane_title}`) surface as a dim, zero-config subtitle under each
  session's label, in the tile bar and the triage queue.
- **Context meters**: a per-tile context-usage meter and a strip-level
  `5h`/`7-day` usage chip, fed by a one-keypress `I` install of a chaining
  statusline wrapper in `~/.claude/settings.json` (any statusline you
  already have keeps running via a `POST /api/statusline/claude` daemon
  ingest endpoint) or, lazily, by tailing a session's transcript when no
  statusline data has arrived yet.
- **`t` — standalone terminal window**: pop the focused live session into
  its own OS terminal window (iTerm2, else Terminal.app) alongside the
  wall's own tmux client — a second attach client, nothing detaches.
- **Restore/close lifecycle in the TUI**: `Enter` restores a dead
  (`restorable`) session, `x x` discards one; `R` restores every restorable
  session in a workspace; closing a worktree session always keeps its
  branch and points you at the web wall's merge/discard/keep flow (that
  finish modal stays web-only for now).
- **Stale-daemon and stale-binary gates**: the launcher detects and swaps a
  daemon left running from a previous version (`GET /api/health` now
  reports `{version, pid}`), and — in a source checkout — rebuilds a wall
  binary that's older than `wall/src`.
- **Daemon API**: session entries gain `message` (the triggering
  Notification hook's text while needs-input), `title` (the live pane
  title), and `context` (`{ usedPercentage, source }`, or `null`); new
  `GET /api/usage` for account-wide 5-hour/7-day rate-limit data.

### Changed

- `claude-garage tui` previously ran on a vendored Dart/nocterm fork
  (nine patches deep, upstream path closed); it's now the Rust/ratatui
  binary described above — same behavior, ~5× less CPU, ~8× less memory,
  and a 3.7× smaller binary (see
  [`openspec/changes/archive/2026-08-31-p9-ratatui-port/verification.md`](openspec/changes/archive/2026-08-31-p9-ratatui-port/verification.md):
  ~5ms input latency, ~2% CPU, 2.4MB binary). The Dart TUI and the nocterm
  fork are removed from the build.

## 0.2.x — the web wall era

The original arc, all in the browser: the terminal bridge and session
lifecycle over tmux; the pit wall (workspace grid, needs-input triage,
macOS notifications); diff review, review mode, and the VS Code jump;
reboot-survives-you restore and `npx claude-garage` packaging; then
worktree sessions, themes, the pit pet, and a VS Code-style dockable grid.
See `git log` for the full history.
