# p7 tasks — VS Code-style grid + UX review fixes

## 1. Grid layout engine (grid-controls)

- [x] 1.1 `lib/layout.js`: rewrite `buildDefault` to place panels in a `ceil(sqrt(n))`-column row-major grid via `addPanel` reference/direction math (D-grid-default); keep the persisted-layout path untouched
- [x] 1.2 `lib/layout.js`: change `reconcile`'s join placement to split the largest panel along its longer axis (`right` when wider than tall, else `below`), plus a one-shot placement-hint mechanism `{referencePanel, direction}` consumed before the heuristic (D-grid-join / D-grid-controls)
- [x] 1.3 `TerminalGrid.jsx`: verify the shipped dockview version's group-maximize API and wire a `toggleMaximize(focusedSessionId)` helper (CSS solo-render fallback if the API is absent)

## 2. Grid controls UI (grid-controls)

- [x] 2.1 `TerminalGrid.jsx`: grid-header toolbar — `+ ▾` menu (new session / new worktree session, auto-label `claude-N`), split-right, split-down, maximize toggle; disabled/no-op when no cell is focused
- [x] 2.2 `SessionCellTab`: add split-right / split-down controls beside the existing hide/pop-out/close, registering the placement hint then calling `createSession`
- [x] 2.3 `App.jsx` keydown: add `\` (split focused cell right) and `m` (maximize toggle), suppressed under the existing terminal-focus rule; exit maximize on `a`-jump focus changes
- [x] 2.4 `HelpOverlay.jsx` + README keybindings table: add `\` and `m`

## 3. Connection resilience (connection-resilience)

- [x] 3.1 `SessionTerminal.jsx`: replace the terminal `[detached]` write with capped-backoff auto-reconnect (0.5s→8s), gated on the session still existing; report `connected` state upward via callback
- [x] 3.2 `TerminalGrid.jsx` (`SessionCellPanel`): render the disconnected overlay ("connection to daemon lost — tmux session still alive") with a manual reconnect button while a cell's socket is down
- [x] 3.3 `App.jsx` + header: connection chip driven by the existing EventSource `open`/`error` events (live / reconnecting), keeping the resync-on-reopen behavior

## 4. Input-mode indicator (input-mode-indicator)

- [x] 4.1 `App.jsx`: `focusin`/`focusout`-derived mode state (`chrome` | session id); header chip renders `keys → garage` / `keys → <label>`
- [x] 4.2 Transient hint toast on terminal focus ("keys now go to <label> — Ctrl+` to return"), auto-dismissing
- [x] 4.3 Footer key strip (`?` `a` `1–9` `\` `m` `r` + mode note), non-focusable, always visible

## 5. Attention badge (attention-badge)

- [x] 5.1 Header needs-input badge: live count across workspaces, click = `jumpToNeedsInput`, quiet "all clear" zero state
- [x] 5.2 `document.title` effect mirroring the count (`(N) claude-garage` ⇄ plain)

## 6. Worktree discard confirm (pit-wall-ui)

- [x] 6.1 `TerminalGrid.jsx` finish toast: two-step armed confirm on `discard` (3s window, destructive styling, branch-naming warning note); merge/keep stay single-click

## 7. Hooks install (hooks-install)

- [x] 7.1 `daemon/src/hooks.js`: `POST /api/hooks/install` — parse-or-refuse settings.json, entry-level dedupe merge, timestamped backup, atomic write (D-hooks-install)
- [x] 7.2 `HooksBanner.jsx`: "install hooks for me" primary action with inline success/error, snippet link secondary, and the `sessions.length > 0` trigger guard
- [x] 7.3 README hook-setup section: document the one-click path first, snippet second

## 8. Onboarding & affordances (pit-wall-ui)

- [x] 8.1 `TerminalGrid.jsx` empty branch: onboarding card (description, 3-step list, add-workspace CTA via new App callback, `?` hint); empty-workspace message points at the rail's `+`
- [x] 8.2 `HelpOverlay.jsx`: status-legend section (`●` `◐` `✓` `○` `⟳` with meanings); `WorkspaceRail.jsx`: tooltips on rail status glyphs
- [x] 8.3 `AddSessionControl.jsx`: inline error text (replaces `!`), `worktree` label
- [x] 8.4 `WorkspaceRail.jsx`: `✕` → armed `unreg` control, padding bump on the rename/open/unreg/add cluster
- [x] 8.5 `index.css`: token bump (faint `#55627a`, dim `#7d8ba1`), global `button { cursor: pointer }`
- [x] 8.6 `App.jsx`: `matchMedia(max-width: 1080px)` auto-collapse of the changes pane with prior-state restore
- [x] 8.7 `App.jsx`: `o` falls back to `openWorkspaceRoot` when no diff file is selected
- [x] 8.8 `ChangesPane.jsx`: "review" button in the pane header sharing the `r` binding's entry path (blur terminal → refetch diff → enter review mode)

## 9. Verification (e2e, per project convention)

- [x] 9.1 Grid: fresh workspace with 4 sessions renders 2×2; split right/down/`\` place correctly; maximize round-trips a custom layout; persisted layouts still load; reset rebuilds balanced grid
- [x] 9.2 Resilience: kill and restart the daemon with terminals on screen — overlays appear, chip cycles live→reconnecting→live, terminals resume without reload; killed session stops retrying
- [x] 9.3 Mode/attention: chip + hint + strip track focus and `Ctrl+``; badge count and `document.title` track a real needs-input transition; badge click jumps
- [x] 9.4 Discard: armed confirm gates a real worktree discard; disarm window works; merge/keep unaffected
- [x] 9.5 Hooks install: run against a settings.json with pre-existing content — merged, deduped on second run, backup written; corrupt file refused untouched; banner guard on empty session list
- [x] 9.6 Record results in `verification.md` per the phase convention
