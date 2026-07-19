# p7 verification — 2026-07-19

Method: real-system e2e against an **isolated daemon** (fake `HOME`, `GARAGE_PORT=4799`,
`GARAGE_SERVE_UI=1` serving the fresh `ui/dist` build, `GARAGE_CLAUDE_CMD` pointed at a
stub so no real Claude sessions were consumed), driven by Playwright against real tmux
sessions in a throwaway git repo. The user's live garage sessions, `~/.garage`, and
`~/.claude/settings.json` were never touched; all test sessions/workspace were cleaned
up afterwards.

## 9.1 Grid (grid-controls) — PASS

- **4 sessions, no persisted layout → true 2×2**: four equal ~486×490 cells, measured
  via `getBoundingClientRect`. *Found & fixed during verification*: the first
  `buildDefault` implementation docked non-row-start panels "right of previous", which
  subdivides the previous panel's own cell instead of aligning under the next column
  (dockview split semantics). Corrected to: first row docks `right`, every later row
  docks `below` its column neighbor (`i - cols`). Re-verified clean.
- **Split** via `\` keybinding: spawned `claude-N` (auto-label), panel placed exactly
  right of the focused cell — the placement hint survived the spawn → SSE refetch →
  reconcile path. *Found & fixed*: focusing the new cell raced App's focus-clamp
  effect (focused id not yet in `sessions`); `refreshSessions` now returns its promise
  and `splitFrom` awaits it before focusing.
- **Maximize round-trip** (`m`): focused cell filled the full grid area (974×1009),
  second press restored the prior arrangement exactly.
- **Persisted layouts load unchanged**: the pre-fix (lopsided) persisted snapshot was
  restored verbatim across two reloads before being cleared — persistence round-trip
  confirmed as a side effect.
- **Reset layout** with 5 sessions: rebuilt as the balanced 3-column default
  (2+2+1, ~323px columns), not a vertical stack.

## 9.2 Resilience (connection-resilience) — PASS

- Killed the daemon with 5 live terminals on screen: header chip flipped to
  "↻ reconnecting…", all 5 cells showed the "connection to daemon lost — tmux session
  still alive" overlay with a reconnect button.
- Restarted the daemon: within the backoff window (≤8s) the chip returned to "● live",
  all overlays cleared, and all 5 terminals resumed streaming — **no page reload**.
- Killed a session via `DELETE /api/sessions/*`: its panel was reconciled out of the
  grid within ~2s (socket disposed with the panel; existence gate stops orphan retries).

## 9.3 Mode / attention (input-mode-indicator, attention-badge) — PASS

- Clicking into a terminal: header chip → `keys → claude-1`, transient hint toast
  ("keys now go to claude-1 — press Ctrl+` to return to garage"), footer strip note
  updated. `Ctrl+`` returned the chip to `keys → garage`.
- Real `Notification` hook POST (with the install token) flipped 5 sessions to
  needs-input: badge showed "● 5 need input", `document.title` became
  "(5) claude-garage". A `Stop` hook cleared it: badge → quiet "all clear",
  title → plain. (Badge-click jump exercised earlier in the session against the
  needs-input cell — focuses and flashes it.)

## 9.4 Discard confirm (pit-wall-ui) — PASS

- Spawned a worktree session (`garage/wtdiscard` branch + worktree created), killed it
  via the two-step ✕ — finish toast appeared.
- First `discard` click: armed ("discard branch?", red) with the warning "deletes
  unmerged work on garage/wtdiscard"; no git operation ran.
- Waiting 3.3s: disarmed back to plain `discard`. (The ✕ close confirm's own 3s
  disarm was also observed incidentally.)
- Armed click-through: toast dismissed, `git branch --list 'garage/*'` → empty,
  worktree directory removed from disk.
- `merge`/`keep` remain single-click (code-inspected; unchanged handlers).

## 9.5 Hooks install (hooks-install) — PASS

- Against a settings.json with pre-existing user content (`model`, a user Stop hook):
  install merged the garage Notification/Stop http hooks + `allowedHttpHookUrls`,
  preserved every pre-existing entry, and wrote a timestamped
  `settings.json.garage-backup-*` alongside.
- Second install: `alreadyInstalled: true`, no duplicate entries (1 http hook per event).
- Corrupt file (`{ broken json !!`): HTTP 422 with an actionable message, file left
  byte-for-byte untouched.
- Banner UI: rendered with "install hooks for me" primary + snippet link secondary
  when idle sessions existed. Empty-session guard verified at the code level
  (`sessions.length === 0 → null`); not reproducible in-browser here because the
  host machine had live garage sessions throughout.

## Also verified in-browser

- Review button in the changes-pane header opens full-screen review mode (shared
  entry path with `r`).
- Help overlay contains the new `\`/`m` rows and the status-legend section
  (all five glyphs with meanings); rail glyphs carry tooltips.
- Changes pane auto-collapsed at 1000px viewport width and restored on widening
  to 1480px.
- Grid-header toolbar (`+ ▾`, ◫, ⬒, ⛶), per-tab split controls, `unreg` control,
  "worktree" label, cursor:pointer, and bumped dim/faint tokens all rendered
  against live data.

## Post-verification follow-up (user testing, same day)

- White vertical strips on every cell's right edge — xterm.js's
  `.xterm-viewport` scrollbar falling back to the browser default (the xterm
  `theme` option colors the canvas only). Kept (it's the scrolled-away-from-
  bottom affordance) but themed: 6px, garage-line thumb on a transparent
  track, dim on hover (`ui/src/index.css`).

## Not covered (accepted gaps)

- First-run onboarding card (`group == null` branch): unreachable on this machine
  without killing the user's live sessions — verified visually in the design mockup
  and by code inspection only.
- Tab-stacked (dockview center-drop) status visibility: not exercised; the tab
  component renders status per-session unchanged, so behavior follows p4's existing
  coverage.
- `vite build` passes; no automated test suite exists in this repo (consistent with
  prior phases).
