# Changelog

## 0.3.1

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
