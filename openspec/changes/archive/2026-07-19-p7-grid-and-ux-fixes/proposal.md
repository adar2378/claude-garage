# p7: VS Code-style grid + UX review fixes

## Why

A specialized UX review (2026-07-19) found that the three moments where a developer decides whether to trust the tool all currently fail: the first five minutes (keyboard mode-trap with no indicator, three empty panes on first run, unexplained status glyphs, hand-merge-JSON hook setup), the first wake-from-sleep (every terminal dies as `[detached]` until a full page reload), and the first worktree finish (one-click `discard` permanently deletes branch work). Separately, the terminal grid's default layout is a vertical stack (`layout.js` only ever splits `below`), so parallel sessions — the product's core moat — render as ever-thinner full-width rows with no split/maximize affordances; VS Code users reach for controls that don't exist. An interactive mockup of all fixes was validated first (claude.ai artifact, 2026-07-19).

## What Changes

**Grid — VS Code-style terminal behavior**
- Default layout becomes a balanced near-square grid (2×2 for 4 sessions) instead of a vertical stack; new sessions split the largest panel along its longer axis instead of always `below`.
- Grid-header toolbar: `+ ▾` spawn menu (plain / worktree session), split right, split down, maximize toggle. Same split/close controls on each cell tab.
- New keybindings: `\` (split focused cell right), `m` (maximize toggle). Maximize uses dockview's group-maximize.

**Critical fixes**
- Input-mode indicator: header chip showing where keys go (`keys → garage` / `keys → <session>`), a transient "Ctrl+` to return" hint on terminal focus, and a persistent footer key strip. Keys pressed while a terminal has focus no longer die silently undiscoverably.
- Terminal WS auto-reconnect with backoff + per-cell "reconnect" overlay replacing the dead `[detached]` end state; header connection chip (live / reconnecting) driven by SSE + WS health.
- Worktree finish `discard` becomes a two-step armed confirm (same "sure?" pattern as session close).

**High fixes**
- Aggregate needs-input badge in the header (click = same as `a`); count mirrored into `document.title`.
- First-run empty state in the grid: product explanation, primary "add workspace" CTA, `?` hint.
- Status-glyph legend section in the help overlay; tooltips on rail glyphs.
- One-click hook install: new daemon endpoint merges the hook snippet into `~/.claude/settings.json` (with backup); banner gains "install for me" and no longer fires when the session list is empty.
- Session-creation errors render as inline text under the form instead of a hover-only `!`.

**Polish (nearly free)**
- `wt` checkbox → "worktree" label; workspace remove control reads `unreg` instead of a second `✕`; bigger hit targets; global `cursor: pointer` on buttons.
- Contrast bump: `--color-garage-faint` → `#55627a`, `--color-garage-dim` → `#7d8ba1`.
- Changes pane auto-collapses to its 32px strip below ~1080px viewport width.
- `o` falls back to opening the workspace root when no diff file is selected (matches README/help copy).

## Capabilities

### New Capabilities
- `grid-controls`: VS Code-style terminal grid — balanced default arrangement, longer-axis placement for joining sessions, split/maximize/spawn controls and their keybindings.
- `connection-resilience`: terminal WebSocket auto-reconnect, per-cell reconnect overlay, and the header connection-state chip.
- `input-mode-indicator`: the keys-routing chip, terminal-focus hint toast, and footer key strip.
- `attention-badge`: aggregate needs-input count in the header and mirrored into the page title.
- `hooks-install`: one-click hook installation via the daemon, replacing manual JSON merging as the primary path.

### Modified Capabilities
- `pit-wall-ui`: the finish prompt's `discard` action requires a two-step armed confirmation (modifies "Worktree finish prompt"); the help overlay gains a status legend and the new keys (modifies "Help overlay"); plus ADDED requirements for the first-run empty state, inline session-creation errors and the "worktree" label, chrome affordance standards (`unreg` vocabulary, hit targets, cursor, contrast tokens), changes-pane auto-collapse, and the `o` workspace-root fallback. (`worktree-sessions` daemon behavior is unchanged — the discard guard is client-side.)

## Impact

- **UI**: `App.jsx` (header chips/badge, keybindings, title effect, pane auto-collapse, `o` fallback), `TerminalGrid.jsx` (toolbar, tab controls, maximize, finish-toast confirm), `lib/layout.js` (grid default + longer-axis reconcile), `SessionTerminal.jsx` (reconnect + focus/blur events), `WorkspaceRail.jsx` (glyph tooltips, `unreg`, targets), `AddSessionControl.jsx` (inline errors, worktree label), `HooksBanner.jsx` (install button, empty-list guard), `HelpOverlay.jsx` (legend, new keys), `index.css` (tokens, cursor, key strip).
- **Daemon**: new `POST /api/hooks/install` in `daemon/src/hooks.js` (settings.json merge + backup).
- **Persistence**: existing dockview layout persistence unchanged; persisted layouts still round-trip — only the *default* build and new-session placement change.
- **Docs**: README keybindings table gains `\` and `m`; hook setup section documents the one-click path.
