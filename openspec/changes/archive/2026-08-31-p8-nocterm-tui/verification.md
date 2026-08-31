# p8-nocterm-tui — end-to-end verification (task group 10)

**Date:** 2026-08-30
**Binary:** `tui/dist/garage-tui-darwin-arm64` (dart compile exe, built from working tree at commit `7727d0f` on `redesign/light-minimal` + uncommitted p8 changes, including the sanctioned `GARAGE_TUI_KEYLOG` instrumentation hook added for 10.2)
**Environment:** macOS 26.6.2 (arm64), tmux 3.7b, Dart SDK 3.12.2, node v22.22.3, Claude Code v2.1.251
**Unit suite at verification time:** `dart analyze` clean, `dart test` 220/220 green (the keylog hook added no test debt)
**Harness:** `tui/test/e2e/run_e2e.sh` (10.1 + 10.2, self-contained, PASS/FAIL per check, non-zero exit on failure). 10.3 was driven interactively with the same tmux send-keys / capture-pane pattern; the exact steps are recorded below.

Method: daemon started by the harness with `GARAGE_CLAUDE_CMD=/bin/zsh` (10.1/10.2) or real `claude` (10.3); TUI runs inside a detached scratch tmux session (`tmux new-session -d -x W -y H`); keys injected with `tmux send-keys` (`-l` literals, `-H` raw hex for escape sequences); assertions read `capture-pane -p` of the outer TUI pane or the inner garage sessions. Byte-exactness targets ran the spike's `keyecho.sh` (`cat -v` in raw mode).

## 10.1 — e2e harness (200×55, zsh sessions)

| # | Check | Result |
|---|---|---|
| 1 | daemon up (`GET /api/health`) | PASS |
| 2 | `PUT /api/workspaces` e2e-alpha / e2e-beta (200) | PASS |
| 3 | `POST /api/sessions` e2e-alpha/main, e2e-beta/keyecho (201) | PASS |
| 4 | keyecho running in inner e2e-beta session | PASS |
| 5 | TUI renders rail with the scratch workspace at 200×55 | PASS |
| 6 | strip shows tabs `1:e2e-alpha 2:e2e-beta` | PASS |
| 7 | digit `2` focuses e2e-beta (keyecho tile content appears in grid) | PASS |
| 8 | digit `1` focuses e2e-alpha (keyecho tile leaves grid) | PASS |
| 9 | Enter engages; chip flips to `keys → e2e-alpha/main` | PASS |
| 10 | typed command executes in the inner session (`echo e2e mark $((21*2))` → `e2e mark 42` in inner capture) | PASS |
| 11 | Ctrl+G disengages; chip back to `keys → garage` | PASS |
| 12 | Alt+Right byte-exact: sent `1b 5b 31 3b 33 43` to the outer pane, inner keyecho capture shows `^[[1;3C` | PASS |
| 13 | Shift+Tab passthrough: sent `1b 5b 5a`, inner keyecho shows `^[[Z` | PASS |
| 14 | `n` spawns (`claude-1` appears in rail; tmux session exists) | PASS |
| 15 | hook-token `POST /api/hooks/claude` Notification (`message: "e2e question"`, cwd = beta dir) accepted (200) | PASS |
| 16 | rail amber ordering: blocked e2e-beta bubbles to tab 1 | PASS |
| 17 | strip shows blocked count badge | PASS |
| 18 | `A` opens triage queue (chip `keys → queue`) | PASS |
| 19 | queue row shows the daemon message text (`e2e question`) | PASS |
| 20 | queue Enter jumps engaged onto the blocked session (chip `keys → e2e-beta/keyecho`) | PASS |
| 21 | hook Stop accepted (200) | PASS |
| 22 | Stop clears needs-input (badge gone, e2e-alpha back to tab 1) | PASS |
| 23 | `q` quits with exit code 0 | PASS |
| 24 | tmux sessions survive TUI quit | PASS |
| 25 | tty restored after quit (`stty -a` sane) | **PASS** (fixed post-run — see deviations) |

(The plain `a` jump — as opposed to the queue's Enter jump — was exercised in 10.3 against real Claude sessions; see below.)

## 10.2 — load test (5 stress sessions + main + claude-1; 6 tiles streaming ≈200 lines/s combined)

Latency = wall-clock from just before `tmux send-keys` of a garage-layer digit to the `GARAGE_TUI_KEYLOG` handled timestamp written inside the key handler (so the numbers *include* tmux client delivery overhead, ~1–2 ms). 5 samples per size.

| Size | Samples (ms) | Median | Assert < 50 ms | TUI %CPU under load (3× 1 s `ps -o %cpu`) |
|---|---|---|---|---|
| 200×55 | 6, 5, 6, 6, 5 | **6 ms** | PASS | 9.7 / 10.4 / 11.0 |
| 250×70 | 6, 6, 6, 5, 5 | **6 ms** | PASS | 10.1 / 11.6 / 11.8 |

| Check | Result |
|---|---|
| 5 stress sessions spawned and streaming | PASS |
| grid streams under load (stress output visible in tiles) | PASS |
| median keypress-to-handled < 50 ms at 200×55 | PASS (6 ms) |
| median keypress-to-handled < 50 ms at 250×70 | PASS (6 ms) |
| `q` quits with exit 0 under load at both sizes | PASS |
| tty restored after quit (both sizes) | **PASS** (fixed post-run — see deviations) |

Notes: a first-ever keypress right after startup measured 96 ms once (cold start; the spike also observed early-frame coalescing) — the design.md risk note ("measure at 250×70 before calling the phase done") is answered: no latency degradation at the larger size. RSS not re-measured (spike: ~70 MB).

Keylog hook: 10-line env-gated addition to `tui/bin/garage_tui.dart` (`GARAGE_TUI_KEYLOG=<file>` appends `<epoch-us> <key>` per garage-layer key). `dart analyze` clean, 220 tests green.

## 10.3 — real-system verification (real `claude`, no `GARAGE_CLAUDE_CMD`)

Two scratch workspaces (`e2e-real-a`, `e2e-real-b`), one session spawned in each via `POST /api/sessions`; TUI at 200×55. No prompt was ever submitted to Claude (composer text was typed but Enter was never pressed on a non-empty composer).

| Check | Result | Evidence |
|---|---|---|
| Claude trust dialog accepted through an engaged tile (Down+Enter) in both sessions | PASS | dialog default `No, exit` → Down+Enter → Claude Code v2.1.251 banner + composer |
| composer renders inside the tile | PASS | `❯` composer line visible in outer tile capture |
| typed text appears in the composer | PASS | typed `hello brave new world` via the TUI; inner capture shows it; cursor_x 2→23 |
| Alt+Left / Alt+Right word-jump moves the inner cursor | PASS | `tmux display-message '#{cursor_x}'`: 23 → 18 (Alt+Left) → 14 (Alt+Left) → 18 (Alt+Right) — exact word boundaries of "new world" |
| `a` jump across workspaces after hook-token Notification | PASS | Notification (cwd = ws-b) → strip `1:e2e-real-b● … ● 1 blocked`; `a` → chip `keys → e2e-real-b/main` (engaged); Stop hook cleared it |
| tile reattach after killing + recreating an inner tmux session | PASS | scratch zsh session killed and recreated with the same name; tile rendered the *new* session's output (`post-reattach-marker` visible, pre-kill content gone) |
| `q` leaves both claude sessions alive | PASS | exit code 0; both `garage/e2e-real-*/main` panes still running claude (pane_current_command `2.1.251`) after quit |
| clean shutdown | PASS | Ctrl+C ×2 sent to each inner session; both claude processes exited; scratch sessions killed |

## Deviations / known gaps

1. **FAIL — terminal not fully restored on `q` quit** (spec tui-wall: "SHALL restore terminal settings on exit"). After quit, the TUI's tty is left with `-icanon -echo` (raw); `ixon ixoff isig` *are* restored (bootstrap.dart's external `stty`). Root cause, confirmed with a minimal Dart repro: nocterm's shutdown (`TerminalBinding._performImmediateShutdown`) cancels the stdin subscription *before* `backend.disableRawMode()`; cancelling Dart's stdin subscription closes the stdin fd, so `stdin.echoMode = true` throws `StdinException: … Bad file descriptor` — swallowed by the backend's catch. Not fixed here (outside the sanctioned change budget for this task group). Candidate fixes: restore termios before cancelling the subscription in the vendored fork, or extend `restoreFlowControl()`/`shutdownTui()` to run an external `stty icanon echo` (which works because it opens `/dev/tty` itself). Practical impact is low — an interactive shell re-asserts its own termios at the next prompt — but it is a real spec deviation. The harness keeps this check (ordered last so a run still yields full metrics).
   **RESOLVED (post-run, reviewed fix):** `restoreFlowControl()` in `tui/lib/bootstrap.dart` now also restores `icanon echo` via the external `stty` (which opens `/dev/tty` itself, immune to the closed-stdin-fd problem). Re-verified on the rebuilt binary: after `q`, `stty -a` shows `icanon`, `echo`, `ixon ixoff` and `isig` all restored. The nocterm shutdown-ordering bug remains an upstream issue candidate; the external restore covers every quit path that reaches `shutdownTui()` or the `finally` backstop.
2. Latency numbers include ~1–2 ms of tmux `send-keys` client overhead (timestamp taken before invoking the tmux client); the in-app number is therefore slightly *better* than reported.
3. The harness measures garage-layer key handling (the spec's "keypress processed" scenario is a workspace digit under load); engaged-layer forwarding latency was not separately instrumented (it shares the same input path upstream of the handler).
4. The real live session `garage/cto-playground/x-com` (pre-existing, untouched) appears as a synthesized third workspace group during runs; the harness clears any *foreign* needs-input state inside its own scratch daemon's in-memory store (Stop hook) before triage-jump checks so a jump can never engage a non-e2e session. No keys were ever sent to it.
5. `verification.md` extras: bare-`a`-jump verified in 10.3 rather than 10.1 (queue open + queue Enter jump verified in 10.1).

## Post-review fixes — mouse clicks + garage typing hint (2026-08-30)

**Gap (found by user testing, missed by the keys-only e2e):** clicks did
nothing anywhere on the wall, and printable typing in the garage layer was
consumed with zero feedback — together the wall read as dead. The 10.1–10.3
harness drove everything through `tmux send-keys` key events (the only mouse
coverage was the wheel), so the spec's click clauses were never exercised:
tui-key-routing "Pressing Enter **(or clicking a tile)** … SHALL engage" and
tui-triage "Pressing `A` **(or clicking the strip badge)** SHALL open" the
queue.

**Fix (uncommitted, same working tree):** a `ClickRegion` render-object
component (`tui/lib/ui/click_region.dart`, modeled on `WheelRegion`'s
paint-offset pattern but riding nocterm's hit-test → MouseTracker annotation
path) fires on the left-button press transition with region-local cell
coords; one coarse region per surface (grid, rail, badge, overlay) resolves
targets through pure mapping functions (`tui/lib/ui/hit_targets.dart`).
GestureDetector was rejected: its tap recognizer fires on release with only
global coords, and MouseTracker dispatches to *every* annotation on the hit
path — nested per-row detectors under a modal barrier double-fire; the
`ClickAbsorber` token (inner region absorbs, outer barrier yields, ordering
guaranteed by child-first annotation insertion) solves that once. No store
changes: every click composes existing transitions
(focusSession/engage/disengage/focusWorkspace/openOverlay/closeOverlay/
jumpToSession). No vendor patches: the framework's mouse routing works.
Behavior: tile click = focus+engage (click on a different tile while engaged
migrates via explicit disengage→focus→engage; click on the engaged tile is a
no-op, clicks are never forwarded to the PTY — only wheel is); rail
session-row click focuses (grid swap-in, never engages), header click
focuses the workspace; blocked-badge click opens the triage queue; queue row
click = select + jump-engage, outside-the-modal click closes; any click
closes the help overlay; overlays block clicks from reaching the wall
underneath. Garage-layer typing of an unbound printable shows the strip
notice "enter engages the focused terminal — keys go to garage now" (~2.5 s,
never amber, timer not restarted within a burst).

**Unit suite:** `dart analyze` clean; `dart test` 231/231 green (11 new in
`tui/test/hit_targets_test.dart`: point→tile exhaustively cross-checked
against `gridCellRect` for n=1..6 over a 17×11 area incl. short-last-row
dead space, point→rail row across headers/sessions/empty groups, triage
modal row offset). Binary rebuilt via `npm run build:tui`.

**Live smoke A — framework-level probe (`tui/test/e2e/click_probe.dart` +
`run_click_probe_smoke.sh`), 100×30 scratch tmux, SGR press/release injected
with `tmux send-keys -H`, no daemon and no ~/.garage touched — 8/8 PASS:**

| Check | Result |
|---|---|
| grid click reports correct region-local coords (`grid:31,9/72x29` for SGR 60,10 past a 28-col rail) | PASS |
| rail row click reports local row 1 through the Container border inset | PASS |
| strip badge region resolves | PASS |
| modal row click absorbed, row offset = 2 (border+padding — validates `triageModalRowOffset` against real layout) | PASS |
| outside-the-modal click reaches the barrier | PASS |
| overlay blocks the wall underneath (grid-area click while open stays `barrier`) | PASS |
| clicks keep firing after an overlay round-trip (press/release state machine resets) | PASS |
| probe renders / quits cleanly | PASS |

**Live smoke B — full-app scratch-daemon smoke
(`tui/test/e2e/run_click_smoke.sh`): authored, NOT run in this session.**
The TUI hardcodes 127.0.0.1:4747 and the user's real daemon was live on that
port (plus a live user TUI attached to it); pausing the daemon was denied by
the session's permission system, and the script — like `run_e2e.sh` —
refuses to run against a live daemon. To execute: stop the real daemon,
run the script (it backs up `~/.garage/state.json`, swaps a scratch state,
spawns only `e2e-click-*` sessions, restores byte-exact and self-checks with
`cmp` on exit), then restart the daemon
(`nohup node daemon/src/index.js >> ~/.garage/daemon.log 2>&1 &` from the
repo root — how the current one runs). It covers: tile-2 click engages
(chip), engaged-tile re-click no-op, engagement migration to tile 1, rail
session/header clicks (focus only), Notification-hook badge → badge click
opens queue → queue-row click jump-engages, outside click closes, help
click-close, typing hint appears and clears, Stop hook, `q` exit 0.

**Cleanup (smoke A):** tmux session list diff-identical to the pre-run
snapshot (`e2e-click-probe` killed; nothing else touched),
`~/.garage/state.json` untouched (SHA-256 `58539e2f…af2117e`, unchanged —
never read or written by the probe), user's daemon and TUI never stopped,
no stray probe processes.

## Cleanup confirmation

- `~/.garage/state.json` restored byte-exact from the pre-run backup — verified with `cmp` and matching SHA-256 (`58539e2f…af2117e`) after 10.1/10.2 (harness does this in its exit trap) and again after 10.3.
- tmux session list after the run is identical to the pre-run snapshot (`diff` clean): all `e2e-*` / `garage/e2e-*` sessions killed, no pre-existing session touched (`garage-spike-*`, `garage/cto-playground/x-com` untouched throughout).
- No stray processes: no `garage-tui-darwin-*` and no `daemon/src/index.js` processes after the run; both real claude sessions were exited cleanly (Ctrl+C ×2) before their tmux sessions were removed.

## Post-review fixes — p8.1 basic-functionality wave (2026-08-31)

**Scope:** the UX-parity reds from user testing — wrapped-text tiles (PTY
never sized), no maximize, no restore, no close, no workspace add, blank
empty states, stale help. Task group 11 in tasks.md; ADDED requirements in
specs/tui-wall (PTY size, maximized tile, empty states) and
specs/tui-key-routing ("p8.1 session lifecycle bindings").

**Binary:** `tui/dist/garage-tui-darwin-arm64` rebuilt from this working
tree. **Unit suites:** `dart analyze` clean; `dart test` 271/271 green
(40 new over the click-fix baseline of 231: maximize state machine,
engage-declines-restorable, restore-all
selection, armed-close, tilde expansion + name derivation, command
mapping, vendored resize patch incl. restart-listener survival);
`npm test` 45/45 (4 new: DELETE `?meta=1` route).

### Resize root cause (the wrapped-text bug) — four stacked drops

Every tile's tmux client stayed at the PtyController spawn default 80×24,
so tmux (`window-size latest`) rendered sessions at 80 cols inside ~68–84
col tiles and lines wrapped twice. Vendored patch 8 (registered in
`tui/vendor/NOCTERM_VERSION`, regression-tested in
`tui/test/terminal_xterm_resize_test.dart`):

1. `_TerminalRenderer` rendered a hardcoded 80×24 ("for now") and reported
   that to `onSizeChange` — the tile's real size never reached
   `_updateSize`. Now constraint-sized via LayoutBuilder.
2. `PtyController.resize` silently no-ops before the process runs; the
   registry starts controllers *after* first layout (staggered attach), so
   the one resize that mattered was dropped forever. Now: dirty-tracked and
   re-pushed on the controller's transition to running (microtask-deferred
   — the resize→notify→setState chain must not run inside build/layout).
3. `PtyController.restart()` → `dispose()` cleared the listener list, so
   any reattach silently detached the TerminalXterm listener and the
   re-push (and every later notification) died. Now: restart preserves
   listeners.
4. `UnixPtyHandler.resize` never resized anything real: it wrote an
   in-band `ESC[8;rows;colst` report into the child's stdin (tmux consumes
   that only as the answer to its own startup `CSI 18 t` query; any other
   time the bytes leak into the pane as typed junk — the `;52;84t`
   fragments seen in shell prompts) plus a `kill -WINCH` at a process
   group whose winsize had never changed. Now: a real TIOCSWINSZ — the pty
   child's tty is resolved (`pgrep -P` + `ps -o tty=`, cached) and
   `stty rows/columns` sets the winsize; the kernel delivers SIGWINCH.
   A resize issued before the child forks retries briefly.

A first suppression attempt for the typed-junk artifact
(`suppressSizeReports`) was reverted for tmux tiles: the attach pty is
pipe-backed, so the `CSI 18 t` handshake is how tmux gets its *initial*
size — suppressing it left clients sizeless and the attach exited. The
knob remains in the fork (default off) with that warning documented.

**Bonus staleness fix found by the harness:** `_refetchSessions` dropped
triggers arriving mid-refetch; if that trigger was the last SSE `sessions`
event, the wall stayed stale indefinitely (a killed session kept rendering
as a dead attach instead of its restorable placeholder). Now
trailing-coalesced (one queued re-run).

### Daemon changes

- `DELETE /api/sessions/<id>?meta=1` — drops only the stored resume
  metadata of a non-live session (worktree record still rides the
  response); plain DELETE (the web UI's call) byte-for-byte unchanged, and
  a live session with `meta=1` is refused (409). Tests:
  `daemon/test/sessions.delete-meta.test.js` (4/4).
- `GARAGE_DIR` env override in `registry.js` (+ `worktrees.js` root) for
  testability — scratch daemons no longer share `~/.garage`. Mirrors
  `GARAGE_PORT`. Default behavior unchanged.
- TUI client/bootstrap honor `GARAGE_TUI_PORT` then `GARAGE_PORT`
  (previously hardcoded 4747) — needed for scratch-port smokes.

### Live e2e — `tui/test/e2e/run_p81.sh` (scratch port 4798, scratch GARAGE_DIR, 200×55): 37/37 PASS, 1 SKIP

| Area | Checks | Result |
|---|---|---|
| scratch daemon + TUI against `GARAGE_TUI_PORT` | 2 | PASS |
| zero-workspace onboarding panel | 1 | **SKIP** — a foreign `garage/*` tmux session (the user's real `cto-playground/x-com`) synthesizes a group, so the wall is never empty on this machine; the empty-workspace hint path IS covered below |
| `w` overlay: open (chip `keys → add workspace`), Esc cancel, bad-path inline error keeps it open, valid path registers derived name `p81-a`, new workspace focused | 6 | PASS |
| empty-workspace hint (`press n for a session, N for a worktree session`) | 1 | PASS |
| spawn ×2; tiles attach | 3 | PASS |
| **resize:** `tmux list-clients` client_width×height == tile inner size 84×52 for BOTH tiles (not 80×24) | 2 | PASS |
| **maximize:** `m` → PTY 170×52 (full grid inner), obscured sibling stays 84×52, `m` again → 84×52 | 3 | PASS |
| **restore:** killed session → restorable placeholder → `]`+Enter → "restoring…" → tmux session live again → placeholder gone (zsh `--resume` falls back to fresh spawn per daemon design) | 5 | PASS |
| **R restore-all:** 2 restorables (killed + injected meta) both live after one `R` | 3 | PASS |
| **x-x close (live):** arm notice names the label, `z` disarms (session survives x·z·x), second armed x kills, "closed" notice | 4 | PASS |
| **x-x close (restorable):** `?meta=1` path — meta gone from `/api/sessions` | 3 | PASS |
| **worktree close:** worktree session closed → "worktree kept:" notice, worktree dir survives, tmux session gone | 6 | PASS |
| `q` exit 0 | 1 | PASS |

### Live e2e — `tui/test/e2e/run_click_smoke.sh` (previously authored-but-blocked): 26/26 PASS

Made fully scratch (port 4799 + scratch `GARAGE_DIR`, `~/.garage` never
read or written — the old state-swap/restore machinery removed) and run
for the first time: every click contract passes (tile click engage,
engaged re-click no-op, engagement migration, rail session/header focus
clicks, badge → queue → row jump-engage, outside-click close, help
click-close, typing hint appear+clear — hint probe key changed `x`→`z`
since `x` is now bound). The p8 click-fix wave is no longer partially
unverified.

### Cleanup confirmation

- `~/.garage` untouched by every run in this wave (scratch `GARAGE_DIR`),
  user's live daemon on 4747 and their TUI never stopped; post-run: no
  `p81-*`/`e2e-click-*` tmux sessions, no scratch daemons, exactly the
  user's one daemon process, `GET /api/health` on 4747 ok.
- The running daemon still executes pre-change code; daemon file changes
  take effect on its next restart (all are additive/env-gated).

### Known gaps / notes

- Zero-workspace onboarding verified in code + conditionally in the
  harness (SKIP on machines with foreign garage sessions — the check runs
  when none exist).
- Restore round-trips exercised with `GARAGE_CLAUDE_CMD=/bin/zsh` and
  injected resume metadata (`claude --resume` against a real conversation
  not exercised here; the daemon path is the same one the web UI uses).
- The user's open TUI keeps running the old binary until relaunched.

## Post-review fixes — engaged cursor + stale-daemon gate (2026-08-31)

**Scope:** two user reports. (1) "I can't see the textfield cursor when
typing in claude code" — the vendored `_TerminalRenderer` painted buffer
cells but never the cursor, and nocterm hides the HOST terminal's real
cursor while running, so an engaged composer showed no caret at all.
(2) `?meta=1` 404s from a pre-upgrade daemon process still serving old
code on 4747 — the launcher happily attached to any healthy daemon.
Task group 11 items 11.10/11.11.

**Binary:** `tui/dist/garage-tui-darwin-arm64` rebuilt from this working
tree. **Unit suites:** `dart analyze` clean; `dart test` 277/277 green
(6 new: `tui/test/terminal_xterm_cursor_test.dart`); `npm test` 47/47
(2 new: `daemon/test/health.test.js`).

### Engaged cursor (vendored nocterm patch 9)

Four stacked drops in `terminal_xterm.dart`, all registered in
`tui/vendor/NOCTERM_VERSION` under patch 9:

1. `_convertCellStyle` computed the cursor's reverse flip and then dropped
   it behind a stale "reverse is not supported in TextStyle" note —
   nocterm's TextStyle DOES carry reverse (SGR 7) all the way through the
   canvas and binding flush.
2. The multi-style line fallback collapsed the whole line to its first
   span's style, which would have erased the one inverted cell anyway —
   the cursor line now renders its real spans through RichText (softWrap
   off); every other line keeps the upstream collapse untouched.
3. An EMPTY cursor cell (codepoint 0 — the composer end-of-input case)
   was never styled at all — now an inverted space (solid-block look).
4. The cursor line matched the view-relative `buffer.cursorY` against an
   absolute buffer index — wrong as soon as scrollback exists; now
   `buffer.absoluteCursorY`.

Contract: `TerminalXterm.showCursor` (default false = upstream); the tile
passes `engaged && !frozen`. The renderer additionally gates on DECTCEM
(`CSI ?25l/h` — apps that hide their cursor keep it hidden) and on the
live view (frozen slice or relative scroll ⇒ no cursor). Content-level
inverse (CellAttr.inverse) deliberately stays unpainted, exactly as
upstream, because the line collapse could flip a whole line. Unengaged
tiles show nothing — a wall of six carets is noise.

### Stale-daemon gate (launcher)

- `GET /api/health` now reports `version` (package.json, read once at
  daemon start — `daemon/src/health.js`) and `pid` (`process.pid`).
- `bin/garage.js` (BOTH `tui` and web paths): after a successful health
  check it compares the reported version to its own; on mismatch or a
  missing version field it prints one line — `garage daemon <v|with no
  version (pre-upgrade)> is stale (launcher vX) — restarting it; sessions
  are untouched (tmux owns them)` — SIGTERMs the daemon by its REPORTED
  pid (never a name-based kill; pre-upgrade daemons that report no pid
  fall back to the port's single LISTEN pid via lsof), waits for the port
  to free (10 s, actionable error otherwise), then starts the new daemon
  exactly like the daemon-absent path and waits healthy.
- `startDetachedDaemon` writes daemon.log under `GARAGE_DIR` when set
  (scratch smokes never touch `~/.garage`); default unchanged.

### Live e2e — `tui/test/e2e/run_p82.sh` (scratch port 4796, scratch GARAGE_DIR, 200×55): 30/30 PASS

| Area | Checks | Result |
|---|---|---|
| stale gate, pid-less shape: stub serving `{"status":"ok"}` (no version/pid — the real pre-upgrade shape) swapped out by the launcher (lsof-by-port), replacement reports current version+pid, TUI up | 5 | PASS |
| stale line printed: `is stale (launcher v…` + `sessions are untouched` (captured from the outer pane's startup scroll) | 2 | PASS |
| launcher daemon.log landed in the scratch GARAGE_DIR | 1 | PASS |
| spawn `/bin/zsh -f` session; rail focus; tile attach | 4 | PASS |
| **cursor:** unengaged tile → ZERO inverse-video cells; Enter engages → EXACTLY ONE (SGR 7 at the prompt cell, from `capture-pane -e` parsed cell-by-cell); typing 6 chars moves it col 53→59 on the same row; still exactly one | 5 | PASS |
| **cursor, frozen:** slow output builds real scrollback (instant bulk output never scrolls the attach client — tmux repaints instead, so `Shift+PageUp` stays live by design), freeze opens (`↓ live` affordance) with NO cursor, `End` snaps live and the cursor returns | 3 | PASS |
| **cursor, alt-screen:** `vim -u NONE` → still exactly one inverted cell, repositioned to the app's cursor; back to one at the shell prompt after `:q!` | 2 | PASS |
| **cursor, disengage:** Ctrl+G → zero inverse cells again | 2 | PASS |
| stale gate, pid shape: versioned stub (`v0.0.1`, reports its own pid) killed via the reported pid, replacement current, stale line names `v0.0.1` | 4 | PASS |
| q quits with exit 0 (both TUI launches) | 2 | PASS |

### Cleanup confirmation

- `~/.garage` never read or written by the harness (scratch GARAGE_DIR
  end-to-end, launcher log included); `state.json` mtime predates the
  runs; the user's daemon on 4747 answered `{"status":"ok"}` before and
  after, never signaled.
- No `p82-tui`/`garage/p82-*` tmux sessions, stubs, or scratch daemons
  after the run; scratch port free.

### Known gaps / notes

- The user's running daemon and open TUI still execute pre-change code
  until relaunched; on the next `claude-garage`/`claude-garage tui` the
  new launcher will restart that daemon once (it reports no version —
  exactly the stale shape stage A covers).
- Frozen-scrollback over tmux attach only accumulates history for output
  slow enough that tmux actually scrolls the client (instant bulk output
  is repainted, not scrolled) — pre-existing emulator/tmux behavior,
  observed while building the freeze checks; noted here, not changed.

## Post-review fixes — p8.3 workspace remove + rail focus marker (2026-08-31)

Two UX additions on top of the p8.1/p8.2 waves.

### `X` workspace remove (armed double-press, registry-only)

- `X` (capital) in the garage layer arms removal of the FOCUSED workspace:
  strip shows `press X again to remove <name> (sessions keep running)`; a
  second `X` within 3 s calls registry-only `DELETE /api/workspaces/<name>`
  (`GarageClient.removeWorkspace` — NEVER the `?sessions=kill` variant);
  any other key disarms. After the refetch the workspace's live sessions
  reappear as the synthesized unregistered group (salience.dart already
  built those; a store transition test now pins it).
- `X` on an unregistered group (registered: false) makes no API call —
  strip explains: `already unregistered — sessions live in tmux; x closes
  them individually`.
- The `x` close machine was generalized into `ArmedAction`
  (tui/lib/state/armed_action.dart); `armed_close.dart` is now a typedef
  alias (`ArmedClose = ArmedAction`) so the p8.1 tests run unchanged. The
  `x` and `X` arms are independent instances and cross-disarm ("any other
  key disarms" spans both).
- Help overlay lists `X X`; spec: new ADDED requirement "p8.3 workspace
  removal and rail focus marker" in tui-key-routing/spec.md.

### Rail focus marker

- The rail now shows WHICH SESSION is focused, not just the workspace: a
  `▸` prefix + bright/bold label (never amber — a focused needs-input row
  keeps its amber hue and only gains bold) on the focused session's row.
  The marker column is reserved on every session row, so rows never shift
  as focus moves; the rail stays one line per row, so `railTargetAt` click
  mapping needed no change.

### Tests

- `dart analyze` clean; `dart test` 284/284 (277 baseline + 7 new:
  `tui/test/armed_action_test.dart` — generic machine, workspace keys,
  x/X independence, alias compatibility — plus wall_store_test additions:
  `X` → `WorkspaceRemoveCommand` mapped and declined as an effect, and the
  removal→unregistered-group salience transition). `npm test` 47/47
  (daemon untouched). dist binary rebuilt
  (`tui/dist/garage-tui-darwin-arm64`).

### Live e2e — `tui/test/e2e/run_p83.sh` (scratch port 4795, scratch GARAGE_DIR, 200×55): 24/24 PASS

| Area | Checks | Result |
|---|---|---|
| scratch daemon + workspace + two `/bin/zsh` sessions + TUI up, p83-a focused via rail digit | 8 | PASS |
| **marker:** exactly one `▸` on screen, on `main`; `]` moves it to `second`; `[` back to `main`; a rail session-row click (SGR mouse injection) moves it to `second` without engaging | 5 | PASS |
| **X arm/disarm:** first `X` arms with the notice naming p83-a; `z` disarms (notice cleared, registration intact) | 3 | PASS |
| **X-X remove:** registration gone from `/api/workspaces`, removal notice shown, BOTH tmux sessions still alive, group resurfaces in the rail as the synthesized unregistered group with its sessions | 6 | PASS |
| **X on unregistered:** explanatory notice, and a second `X` still removes nothing | 2 | PASS |
| q quits with exit 0 | 1 | PASS |

### Cleanup confirmation

- `~/.garage` never read or written (scratch GARAGE_DIR end-to-end;
  `state.json` mtime predates the run); the user's daemon on 4747 answered
  `{"status":"ok"}` after the run, never signaled; no `p83-tui` /
  `garage/p83-*` tmux sessions remain; scratch port free.

## p8.4 — kill-all removal confirm (2026-08-31)

`K` while the `X` remove arm is live confirms removal WITH session kill
(spec tui-key-routing "p8.4 kill-all removal confirm", task 11.14).

### Implementation

- `GarageClient.removeWorkspace` gains `killSessions` → appends
  `?sessions=kill` and returns the daemon's
  `{removed, killedSessions, failedSessions?}` body (null on the
  registry-only 204). `X`-`X` still calls the plain variant.
- The `ArmedAction` machine stays generic: the `K` branch is caller-level
  (`tui/lib/state/workspace_remove.dart`). `confirmKillTarget` confirms
  only a live, unexpired `X` arm (an expired arm is disarmed, never
  re-armed by `K`); `K` is checked BEFORE the cross-disarm pass in the key
  handler so it doesn't disarm the arm it confirms, and outside an arm it
  falls through as an ordinary unbound key (typing hint). Unregistered
  groups still can't arm (p8.3), and a mid-arm refetch that unregistered
  the target is re-checked before firing.
- Arm notice with `n > 0` live sessions:
  `press X again to remove <name> (sessions keep running) · K to also
  kill its <n> sessions` (clause omitted at zero — p8.3 wording
  unchanged). Confirm notice: `removed <name> · killed <n> sessions`,
  with any `failedSessions` named
  (`· failed to kill: <id>, …`). Help overlay adds
  `X K  remove focused workspace AND kill its sessions`.

### Tests

- `dart analyze` clean; `dart test` 295/295 (284 baseline + 11 new in
  `tui/test/workspace_remove_test.dart`: K confirms only a live arm and
  consumes it, never arms/re-arms, unbound after X-X or expiry, only
  registered workspaces can ever reach a confirm, `garageCommandFor('K')`
  is null, arm/confirm wording incl. zero-session and failure shapes).
  `npm test` 47/47 (daemon untouched — `?sessions=kill` already existed).
  dist binary rebuilt (`tui/dist/garage-tui-darwin-arm64`).

### Live e2e — `tui/test/e2e/run_p84.sh` (scratch port 4796, scratch GARAGE_DIR, 200×55): 25/25 PASS

| Area | Checks | Result |
|---|---|---|
| scratch daemon + 3 workspaces (p84-keep ×2 sessions, p84-kill ×2, p84-empty ×0) + TUI up | 10 | PASS |
| **X-X stays registry-only:** arm notice carries `· K to also kill its 2 sessions`; confirm unregisters p84-keep and BOTH `/bin/zsh` tmux sessions stay alive | 4 | PASS |
| **bare K is unbound:** with no arm, `K` removes nothing and kills nothing | 2 | PASS |
| **X then K kills for real:** p84-kill unregistered, strip shows `removed p84-kill · killed 2 sessions`, tmux sessions alpha AND beta gone (`tmux has-session` fails for both) | 5 | PASS |
| **zero sessions:** p84-empty's arm notice is the plain p8.3 wording with no `K` clause; `z` disarms leaving it registered | 3 | PASS |
| q quits with exit 0 | 1 | PASS |

`run_p83.sh` re-run on the new binary: 24/24 PASS (p8.3 flow
regression-free — its notice check is a prefix of the new wording).

### Cleanup confirmation

- `~/.garage` never read or written (`state.json` mtime predates both
  runs); the user's daemon on 4747 answered `{"status":"ok"}` after the
  runs, never signaled; no `p84-tui` / `garage/p84-*` (or p83) tmux
  sessions remain; scratch ports free.
