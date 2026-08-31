# Verification: p10-views-and-subtitles

Scope of this pass: **Part 1** (daemon poller title-diff fix + tests) and **task
group 4** (verification). `wall/src` was not modified (feature-complete per
waves 1–3); any bug found in it during e2e is reported below, not fixed.

Environment: macOS (darwin/arm64), Node 22, tmux, cargo. Fresh build via
`npm run build:tui` immediately before this pass. Daemon on the real port
4747 (`~/.garage`) was never touched by any command in this pass.

## Part 1 — daemon poller title-diff fix

### Problem

`daemon/src/poller.js` only emitted `sessions-changed` when the garage
session-id **set** changed (spawn/death). A pane-title-only change
(`tmux select-pane -T`, e.g. Claude Code's OSC title) never touches that set,
so it sat unpushed to the wall's SSE stream (`GET /api/events` →
`daemon/src/events.js`) until some unrelated refetch happened to notice it —
subtitles (spec `session-status` "Pane title exposure", `tui-wall` "Auto-
subtitle in the tile bar") could go stale indefinitely.

### Fix

- Extended the poller's own `list-panes` call (`getPanePids`, already made
  once per tick for the pid→status join) to also capture `#{pane_title}` —
  **no new tmux invocation**, kept to one `list-panes` per tick daemon-wide.
- Extracted the id-set diff into a pure, exported `diffSessionState(sessions,
  panePids, hostname, prevIds, prevTitles)` and extended it to also compare
  each live session's **normalized** title (via `tmux.js`'s existing
  `normalizeTitle`) against the previous tick's map. A title OR an id-set
  change now both set `changed = true`, driving the same single
  `sessions-changed` emit.
- `lastTitles` is a small `Map<sessionId, string|null>` rebuilt wholesale
  from the current tick's live sessions every tick (never merged), so a dead
  session's entry is dropped automatically — memory-trivial.
- Comparing **normalized** (not raw) titles avoids a false-positive change
  when a pane's title flaps between different tmux-default noise strings
  (e.g. hostname ↔ bare shell name) that both normalize to `null`.

Files: `daemon/src/poller.js`, `daemon/test/poller.test.js`.

### Unit test results

`node --test daemon/test/*.test.js ui/test/*.test.js` (via `npm test`):

| Suite | Result |
|---|---|
| Baseline (pre-existing) | 60/60 pass |
| New: `diffSessionState` title-change → changed | pass |
| New: unchanged id set + unchanged title → no change | pass |
| New: title that normalizes the same (hostname↔shell noise) → no change | pass |
| New: brand-new session (never tracked) → changed via id set | pass |
| New: session death (in prevIds, absent from sessions) → still changed | pass |
| New: session with no matching pane entry → normalizes to null, no throw | pass |
| **Total** | **66/66 pass** |

Isolation check: re-ran the daemon suite and `wall/test/e2e/run_p81.sh`
against the **unmodified** `poller.js` (via `git stash` on just
`daemon/src/poller.js`/`poller.test.js`, then `git stash pop` to restore) to
confirm nothing else in this pass depends on or regresses from the fix. No
change in behavior other than the intended one.

## Task 4.1 — existing wall/test/e2e suites (fresh build, no wall/src changes)

Built via `npm run build:tui`; `cargo test --release` in `wall/`: **331
passed, 0 failed** (lib unit tests) + **1 passed** (`tests/injection.rs`).

| Suite | Checks | Result |
|---|---|---|
| `run_e2e.sh` | 35 | ALL PASS |
| `run_p81.sh` | 37 | PASS (see flake note below) |
| `run_p82.sh` | 30 | ALL PASS |
| `run_p83.sh` | 24 | ALL PASS |
| `run_p84.sh` | 25 | ALL PASS |
| `run_click_smoke.sh` | 26 | ALL PASS |

One test-script edit was required to keep `run_p81.sh` accurate (not a
wall/src fix — see "Geometry note" below); everything else in `wall/src` is
unmodified.

### Geometry note — `run_p81.sh` updated, not a regression

p10's group frame renders around **any** view with 2+ sessions, including a
workspace's plain default view when it simply has two sessions and no manual
grouping (spec `tui-views` "View strip and group frame": "A view containing
2 or more sessions SHALL render a subtle group frame..."). `run_p81.sh`
predates p10 and hardcoded pre-framing PTY sizes for its 2-session
`p81-a/{main,second}` pair (84x52 unframed → now 83x50 framed; maximized
170x52 → now 168x50; both empirically verified against this exact binary at
200x55 before editing the script). Updated the three affected assertions
and their explanatory comment in `run_p81.sh`; nothing in `wall/src` changed
for this. Confirmed stable (re-run twice, 37/37 both times, after this
edit).

### Flake found — pre-existing, NOT caused by p10 (reported, not fixed)

`run_p81.sh` section 6 ("R restores every restorable in the workspace")
intermittently failed at the check `restorable placeholder before R`:
after `add_meta second` + `add_meta ghost` + `tmux kill-session second`, the
tile for `second` sometimes gets stuck rendering

```
✕ attach dead
attach exited (code 1) — gave up after 4 reattach attempts
```

instead of transitioning to the `⟳ restorable` placeholder, even though
`GET /api/sessions` already reports `"restorable": true` for it (confirmed
via the failing run's own `fail-sessions.json`). `wall/src/ui/tile.rs`'s
`placeholder_for` checks `!v.session.live()` (i.e. the API-sourced status)
**before** `dead_reason`, so once the wall's own session list reflects
`restorable`, the tile should render as restorable regardless of a prior
failed-attach `dead_reason` — the tile isn't picking that up in time here.

**Isolation performed:** re-ran this exact suite against the daemon with
**this change's poller.js fix reverted** (`git stash` on
`daemon/src/poller.js`/`poller.test.js` only, `wall/src` untouched) — the
flake reproduced identically (2/2 failed at the same check). This rules out
Part 1's poller change as the cause.

Observed rate this session: roughly half of ~9 runs failed at this exact
check, worsening as the run went on (system load average 5–8 throughout,
unrelated background load on this machine) then clearing back up on a later
clean run — consistent with a timing-sensitive race between the wall's
local PTY reattach-retry-exhaustion and its SSE-driven session refetch
picking up the daemon's `restorable` status, rather than a hard logic bug.
Not something I changed or should fix (owned by `wall/src`, out of scope
for this pass) — reported here precisely per the task's instructions.
Recommend the `wall/src` owner look at the `dead_reason` vs. live-status
precedence/refresh interaction in the reattach/reconcile path.

## Task 4.2 — new `run_p10.sh`

Wrote `wall/test/e2e/run_p10.sh` in the established pattern (scratch port
4797, scratch `GARAGE_DIR` for **both** the daemon and the TUI — the wall
needs it too since `wall.json`'s location is resolved independently via
`persistence.rs`; sessions named `garage/p10e2e-*`; self-cleaning trap;
PASS/FAIL per check). **60/60 checks pass**, confirmed stable across two
full runs; tmux/daemon fully cleaned up after each (verified via `tmux ls`,
`lsof`).

Coverage, end to end against the real binary:

| # | Scenario | Result |
|---|---|---|
| 1 | Single view: no view-strip row reserved (PTY 170x52 proxy) | PASS |
| 2 | Auto-subtitle pushed via `tmux select-pane -T` within ~5s **with no spawn/restart** (the exact behavior Part 1 enables) | PASS (observed ~2s, one poller tick) |
| 3 | `d` detach: view strip appears (`main`/`alpha`), "detached alpha" notice, group frame absent on the now-solo focused view (PTY 170x51, not 83x50) | PASS |
| 4 | `Tab` cycles the focused workspace's views; grid content swaps with it (echoed markers) | PASS |
| 5 | `d` rejoin: single view again, framed pair (PTY 83x50 both) | PASS |
| 6 | `D` picker → new group "backend" (moved alpha → backend); `D` again → move beta into the same existing group (moved beta → backend); frame present once it holds 2 sessions (PTY 83x50 both), default view vanished (no strip) | PASS |
| 7 | Cross-workspace: `watched` detached into its own background view of `p10e2e-cross`; hook-token Notification (targeting `watched`'s unique worktree dir, so the cwd fail-open match can't also catch its sibling `other`) flags only `watched`; `a`-jump crosses BOTH workspace and view and lands engaged; view strip shows the amber dot on `watched`; Stop hook clears it | PASS |
| 8 | Persistence: quit → relaunch against the same scratch `GARAGE_DIR`; both groups' assignments survive in `wall.json` (which lives only in the scratch dir, confirmed absent/unrelated in the real `~/.garage`) | PASS |

### Wall bug found during 4.2 (p10-specific) — FIXED

**A workspace whose default view is fully vacated (every session moved into
named views) gets stuck on that empty default view after a fresh TUI
process start, with no keyboard escape.**

**Status: FIXED** (state layer, `wall/src/state/store.rs` +
`wall/test/e2e/run_p10.sh`; `wall/src/state/views.rs` needed no change —
`compute_views`'s existing display order, default-first-when-non-empty then
named views in stored order, is exactly the order the fix redirects onto).

Fix, principled rather than a patch-over:

- **(a)** A new `refocus_vacated_views` pass runs inside `reconcile` (so on
  every `load_views` call AND every live `sessions_fetched`/
  `workspaces_fetched`/`status_changed` refetch, right after view pruning
  and the existing dangling-focused-view-name cleanup): for every workspace,
  if its currently-resolved focused view has zero members but some other
  view of that workspace is non-empty, focus is redirected to the first
  non-empty view in `compute_views`' own display order. A workspace whose
  sessions are ALL gone is left alone (nothing to redirect to — the
  legitimately-empty-workspace hint is untouched). This fixes both the
  restart shape (a cold `load_views` — `focused_view` is never persisted, so
  a fresh process otherwise assumes `DEFAULT_VIEW` for every workspace
  regardless of whether it's since been vacated) and the equivalent live
  shape (a session dying kills the last member of the *currently-focused*
  view while the default is *also* still empty).
- **(b)** `cycle_view` (`Tab`) no longer bails out just because fewer than
  two non-empty views exist. It now only no-ops when there is truly nowhere
  else to go: no focused workspace, a workspace with zero sessions at all,
  or exactly one real view with focus already on it. If focus is currently
  on a view that isn't in the real-views list at all (i.e. it's empty), Tab
  now lands on the first real view even when that's the *only* one — never
  a dead end, independent of and in addition to (a).
- **(c)** The "no sessions in this workspace" / "press n" hint
  (`wall/src/runtime.rs` `draw_frame`) is gated purely on
  `state().gridded_session_ids.is_empty()` once a workspace has a group at
  all — it has no separate opinion about `views`/`focused_view`. Since (a)
  keeps that field populated with the redirected view's members, the hint
  can never wrongly fire for an empty default sitting next to a populated
  named view; it still fires correctly for a workspace with genuinely zero
  sessions. Verified at the state layer (the field the render decision
  reads) since `wall/src/runtime.rs` is outside this fix's ownership.

Unit tests added (`wall/src/state/store.rs`, all passing):

| Test | Covers |
|---|---|
| `p10_bug_restart_shape_load_views_with_everything_in_a_named_view_focuses_it` | (a), the exact restart shape asked for: `load_views` with every session already assigned to a named view lands `focused_view_name` on that view |
| `p10_bug_reconcile_refocuses_when_a_live_death_vacates_the_focused_view_and_default_is_also_empty` | (a), live shape: a session dying empties the focused named view while default is also empty — falls through to the other real view, not the trap |
| `p10_bug_refocus_is_a_no_op_when_the_focused_view_already_has_members` | (a) guard: an already-correct focus is never disturbed |
| `p10_bug_a_truly_empty_workspace_is_left_on_the_default_not_redirected` | (a) guard: a genuinely empty workspace keeps rendering the correct empty-workspace path |
| `p10_bug_cycle_view_escapes_an_empty_focused_view_even_with_only_one_real_view_left` | (b), direct unit test of `cycle_view`'s own dead-end guard, independent of (a) |
| `p10_bug_cycle_view_stays_put_with_a_single_real_view_already_focused` | (b): one real view, already there — correct no-op (version unchanged) |
| `p10_bug_vacated_default_view_never_renders_the_empty_workspace_hint` | (c): `gridded_session_ids` stays non-empty after the restart shape, contrasted against a genuinely empty workspace |

`run_p10.sh`'s persistence section (#8) was rewritten to assert the real
recovery instead of documenting the bug and routing around it: after
relaunch, focusing the fully-vacated-default workspace now lands directly on
the populated "backend" group with both sessions gridded and framed (PTY
83x50, no view strip — only one real view exists) with NO spawn workaround
and no phantom "no sessions in this workspace" hint; a subsequent `Tab`
press is asserted to be a correct no-op (still on "backend", nothing
disturbed). Re-run twice end to end against the real binary: **60/60 checks
pass both times**, tmux/daemon fully cleaned up after each run (`tmux ls`,
`lsof`), the real daemon on port 4747/`~/.garage` untouched throughout.

`cargo test --release` in `wall/`: **338 passed** (lib, includes the 7 new
tests above on top of the prior 331) + **1 passed** (`tests/injection.rs`) —
**339/339, 0 failed**. `cargo clippy --all-targets -- -D warnings`: clean.
`cargo build --release`: clean.

Original repro notes, for reference (now fixed, see above):

Repro: create workspace with 2 sessions; `D`-move both into a new group
"backend" (default `main` now has 0 sessions); quit; relaunch the TUI
against the same `GARAGE_DIR`. Focusing that workspace shows "no sessions in
this workspace" (the phantom default view) — confirmed correct in isolation
per `with_focused_workspace`'s `DEFAULT_VIEW` fallback, since `focused_view`
(which view is *currently shown*) is deliberately not part of `wall.json`'s
persisted schema, only session→view *assignments* are. The bug: from here,
**neither `Tab` nor `]` can reach "backend"** — `WallStore::cycle_view`
requires `views_for(workspace).len() >= 2`, but `views_for` doesn't count a
0-session view (matches the "main itself vanishes when every session is
detached" semantics), so with only "backend" real, there's nothing to
"cycle" between; `]` no-ops too since `gridded_session_ids` is empty. The
only escape found was spawning a new session (`n`), which puts a live
session back into `main`, making it a real second view again.

This was a genuine reachability gap in `wall/src`'s view-focus persistence
(the assignments-only design is fine per spec; the *unreachable stuck state*
on restart was not what the persistence spec scenario "Restart preserves
groups" intended). At the time this was written, `run_p10.sh`'s persistence
check for this exact shape documented the bug in place (`Tab` was asserted
to be a no-op there) and routed around it with an `n`-spawn workaround to
still verify the group's data actually survived. That workaround has since
been removed — see "Status: FIXED" above — and the script now asserts the
real recovery directly.

## Safety verification

- Both scratch daemon and scratch TUI always received `GARAGE_DIR` pointed
  at a per-run `mktemp` directory; `wall.json`'s presence in that scratch
  dir was asserted directly, and its absence/unrelatedness in the real
  `~/.garage/wall.json` was asserted too.
- The real daemon on port 4747 (pid unrelated to any of these runs) was
  checked before and after every suite (`curl :4747/api/health`) and never
  touched.
- Every suite refuses to start if its port is already bound, and their
  cleanup traps (`EXIT INT TERM`) only ever touch tmux sessions matching
  their own prefix (`p10e2e-*` / `garage/p10e2e-*` for the new suite).
  Verified via `tmux ls` and `lsof` after every run in this pass: no leaked
  tmux sessions, daemon processes, or bound ports.
