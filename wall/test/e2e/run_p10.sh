#!/usr/bin/env bash
# p10-views-and-subtitles — e2e harness for the garage TUI (task 4.2).
#
# Covers (specs tui-views, tui-wall, session-status):
#   1. Single view: no view strip renders (PTY size proxy — see geometry
#      note below); the daemon-provided subtitle in the tile bar.
#   2. `d` detach: strip appears, "detached <label>" notice, group frame
#      absent on the now-solo focused view.
#   3. Tab cycles the focused workspace's views.
#   4. `d` rejoin: back to a single view, no notice.
#   5. `D` picker: move focused session into a new group ("moved <label> →
#      <view>" notice); move a second session into that same group — the
#      group frame renders once it holds 2+ sessions.
#   6. Cross-workspace: a session detached into a background view of
#      workspace B gets an amber dot on the view strip once B is focused; a
#      background hook-token Notification then `a`-jump switches BOTH
#      workspace and view and lands engaged.
#   7. Auto-subtitle push: `tmux select-pane -T` on a live session's pane is
#      picked up and rendered WITHOUT any spawn/restart/keypress — this is
#      exactly the daemon poller title-diff fix from part 1 (a
#      pane-title-only change doesn't touch the session-id set, so it only
#      reaches the wall if the poller diffs titles too).
#   8. Persistence: quit, relaunch against the same scratch GARAGE_DIR,
#      view assignments survive; wall.json lives ONLY in the scratch
#      GARAGE_DIR.
#
# Geometry note (why the PTY-size checks below use the numbers they do):
# p10's group frame renders around ANY view (default included) once it has
# 2+ sessions — not just manually-built groups — so a plain two-session
# workspace with no grouping at all is framed. All numbers here were
# verified empirically against this exact binary (200x55, rail 28, one
# bottom status strip) before being written into the script — see
# openspec/changes/p10-views-and-subtitles/verification.md for the
# derivation. Framing subtracts 2 from each dimension of the grid area;
# the one-line view strip (2+ views) subtracts 1 more row.
#
# Safety contract (fully scratch — the user's live daemon on 4747 and
# ~/.garage are never touched):
#   - runs its own daemon on a SCRATCH PORT (GARAGE_E2E_PORT, default 4797)
#     with a SCRATCH GARAGE_DIR; BOTH the daemon AND the TUI get GARAGE_DIR
#     — the TUI needs it too, since wall.json's location is GARAGE_DIR-aware
#     independently of the daemon (wall/src/state/persistence.rs);
#   - refuses to run when something already listens on the chosen port;
#   - only tmux sessions named p10e2e-tui / garage/p10e2e-* are created or
#     killed; every notification hook call targets a directory unique to
#     one session (a worktree session's own dir), never a shared workspace
#     dir, so it can never over-notify a sibling session.
#
# Exits non-zero on the first failed check; prints PASS/FAIL per check.

set -u -o pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../../.." && pwd)
# Binary under test: default is the Rust dist binary (wall/dist); override
# with GARAGE_TUI_BIN.
TUI_BIN=${GARAGE_TUI_BIN:-$REPO/wall/dist/garage-wall-darwin-$(node -p 'process.arch' 2>/dev/null || echo arm64)}
WORK=${E2E_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/garage-wall-p10.XXXXXX")}
RESULTS=$WORK/results.txt
mkdir -p "$WORK"
: > "$RESULTS"

PORT=${GARAGE_E2E_PORT:-4797}
BASE=http://127.0.0.1:$PORT
SCRATCH_GARAGE_DIR=$WORK/garage-home
mkdir -p "$SCRATCH_GARAGE_DIR"
DAEMON_PID=""
OUTER=p10e2e-tui

CHECKS_RUN=0
note() { printf '     %s\n' "$*"; }
pass() { CHECKS_RUN=$((CHECKS_RUN+1)); printf 'PASS %s\n' "$*" | tee -a "$RESULTS"; }
fail() {
  CHECKS_RUN=$((CHECKS_RUN+1)); printf 'FAIL %s\n' "$*" | tee -a "$RESULTS"
  tmux capture-pane -p -t "=$OUTER:" > "$WORK/fail-capture.txt" 2>/dev/null
  curl -s -m 2 "$BASE/api/sessions" > "$WORK/fail-sessions.json" 2>/dev/null
  curl -s -m 2 "$BASE/api/workspaces" > "$WORK/fail-workspaces.json" 2>/dev/null
  exit 1
}
check() { local desc=$1; shift; if "$@"; then pass "$desc"; else fail "$desc"; fi; }

wait_for() { # wait_for <timeout> <command...>
  local timeout=$1 t0; shift
  t0=$(date +%s)
  while true; do
    if "$@"; then return 0; fi
    [ $(( $(date +%s) - t0 )) -ge "$timeout" ] && return 1
    sleep 0.2
  done
}

cap()         { tmux capture-pane -p -t "=$OUTER:" 2>/dev/null; }
outer_has()   { cap | grep -qF -- "$1"; }
outer_lacks() { ! outer_has "$1"; }

# The tile attach client's size for a garage session ("<w>x<h>", empty if no
# client attached — e.g. its view isn't currently focused/gridded).
client_size() { tmux list-clients -t "=$1" -F '#{client_width}x#{client_height}' 2>/dev/null | head -1; }
client_size_is() { [ "$(client_size "$1")" = "$2" ]; }

# Focus a workspace via its rail digit (tmux is global: the user's real
# garage/* sessions synthesize foreign groups, so the index is dynamic).
focus_ws() { # focus_ws <name>
  local idx
  idx=$(cap | grep -oE "[0-9]+ $1" | head -1 | awk '{print $1}')
  [ -n "$idx" ] || fail "workspace $1 not found in the rail"
  tmux send-keys -t "=$OUTER:" -l "$idx"
}

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p10e2e-tui|garage/p10e2e-)' \
    | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done
  if [ -n "$DAEMON_PID" ]; then kill "$DAEMON_PID" 2>/dev/null; wait "$DAEMON_PID" 2>/dev/null; fi
  # Fully scratch: state lived in $SCRATCH_GARAGE_DIR; ~/.garage untouched.
  exit "$rc"
}
trap cleanup EXIT INT TERM

# ── Preflight ────────────────────────────────────────────────────────────
[ -x "$TUI_BIN" ] || { echo "TUI binary missing: $TUI_BIN"; exit 2; }
for tool in tmux node curl python3 git; do
  command -v "$tool" >/dev/null || { echo "missing tool: $tool"; exit 2; }
done
if curl -s -m 2 "$BASE/api/health" | grep -q ok; then
  echo "refusing to run: something is already listening on $BASE (set GARAGE_E2E_PORT)"; exit 2
fi
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p10e2e-tui|garage/p10e2e-)' \
  | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done

# ── Daemon up (scratch port + scratch GARAGE_DIR, zsh sessions) ──────────
GARAGE_PORT=$PORT GARAGE_DIR=$SCRATCH_GARAGE_DIR GARAGE_CLAUDE_CMD=/bin/zsh \
  node "$REPO/daemon/src/index.js" >"$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
check "scratch daemon up on :$PORT with scratch GARAGE_DIR" \
  wait_for 10 sh -c "curl -s -m 1 $BASE/api/health | grep -q ok"
TOKEN=$(curl -s "$BASE/api/hooks/snippet" | python3 -c 'import json,sys;print(json.load(sys.stdin)["hooks"]["Notification"][0]["hooks"][0]["url"].split("token=")[1])')
hook() { curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/hooks/claude?token=$TOKEN" \
  -H 'content-type: application/json' -d "$1"; }

# ── Workspaces + sessions ─────────────────────────────────────────────────
put_ws() { curl -s -o /dev/null -w '%{http_code}' -X PUT "$BASE/api/workspaces" \
  -H 'content-type: application/json' -d "{\"name\":\"$1\",\"dir\":\"$2\"}"; }
spawn() { curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/sessions" \
  -H 'content-type: application/json' -d "{\"workspace\":\"$1\",\"label\":\"$2\"$3}"; }
# Body-returning variant, for reading the worktree path back out of the
# response rather than reconstructing it (mktemp's $TMPDIR can carry a
# trailing slash into $WORK, so a hand-built path can literally
# double-slash — a byte-different string from what the daemon reports as
# the session's `dir`, which silently breaks the hooks' exact-match cwd
# lookup; reading it back from the API sidesteps the whole question).
spawn_body() { curl -s -X POST "$BASE/api/sessions" \
  -H 'content-type: application/json' -d "{\"workspace\":\"$1\",\"label\":\"$2\"$3}"; }

WS_SOLO=$WORK/ws-solo         # single-session workspace: no-strip + subtitle
WS_GROUP=$WORK/ws-group       # two-session workspace: d/Tab/D flows
WS_CROSS=$WORK/ws-cross       # two-session workspace (a worktree pair): cross-view a-jump
mkdir -p "$WS_SOLO" "$WS_GROUP"
# WS_CROSS needs to be a real git repo — "watched" spawns as a worktree
# session so its dir is unique from "other"'s (both in the same registered
# workspace otherwise share one dir, which would make the hook's cwd
# fail-open match BOTH sessions instead of only the intended one).
mkdir -p "$WS_CROSS"
git -C "$WS_CROSS" init -q
git -C "$WS_CROSS" -c user.email=p10@e2e -c user.name=p10 commit -q --allow-empty -m init

check "PUT workspace p10e2e-solo"  test "$(put_ws p10e2e-solo "$WS_SOLO")"   = 200
check "PUT workspace p10e2e-group" test "$(put_ws p10e2e-group "$WS_GROUP")" = 200
check "PUT workspace p10e2e-cross" test "$(put_ws p10e2e-cross "$WS_CROSS")" = 200
check "spawn p10e2e-solo/lonely"  test "$(spawn p10e2e-solo lonely '')"  = 201
check "spawn p10e2e-group/alpha"  test "$(spawn p10e2e-group alpha '')"  = 201
check "spawn p10e2e-group/beta"   test "$(spawn p10e2e-group beta '')"   = 201
check "spawn p10e2e-cross/other" test "$(spawn p10e2e-cross other '')" = 201
WATCHED_BODY=$(spawn_body p10e2e-cross watched ',"worktree":true')
WATCHED_DIR=$(echo "$WATCHED_BODY" | python3 -c 'import json,sys;print(json.load(sys.stdin)["worktree"]["path"])')
check "spawn p10e2e-cross/watched (worktree)" test -n "$WATCHED_DIR"
check "watched's worktree dir exists" test -d "$WATCHED_DIR"

# Distinctive echoed markers per session: the rail lists every session's
# LABEL regardless of which view is focused, so a bare `outer_has '<label>'`
# never proves a tile's CONTENT is actually gridded. These markers only
# appear when that session's live terminal is actually rendered.
mark() { tmux send-keys -t "=garage/$1/$2:" -l "echo ${2}-mark-$3"; tmux send-keys -t "=garage/$1/$2:" Enter; }
mark p10e2e-solo  lonely  1
mark p10e2e-group alpha   2
mark p10e2e-group beta    3
mark p10e2e-cross other   4
mark p10e2e-cross watched 5

# ── Launch the TUI (200x55) against the scratch daemon — GARAGE_DIR too, ─
# not just GARAGE_TUI_PORT: wall.json's location is resolved independently
# of the daemon (persistence.rs reads GARAGE_DIR itself).
cat > "$WORK/launch.sh" <<EOF
#!/bin/sh
"$TUI_BIN"
echo \$? > "$WORK/tui-exit"
sleep 120
EOF
chmod +x "$WORK/launch.sh"
launch_tui() { # launch_tui <tmux-session-name>
  tmux new-session -d -x 200 -y 55 \
    -e GARAGE_TUI_PORT="$PORT" -e GARAGE_DIR="$SCRATCH_GARAGE_DIR" \
    -s "$1" "$WORK/launch.sh"
}
launch_tui "$OUTER"
check "TUI launches against the scratch daemon (strip chip renders)" \
  wait_for 15 outer_has 'keys → garage'
check "p10e2e-solo appears in the rail" wait_for 20 sh -c \
  "tmux capture-pane -p -t '=$OUTER:' | grep -qE '[0-9]+ p10e2e-solo'"

# ── 1. Single view: no strip; PTY fills the un-inset, un-strip'd grid ────
focus_ws p10e2e-solo
check "focused p10e2e-solo (lonely's own content is gridded)" \
  wait_for 15 outer_has 'lonely-mark-1'
check "single view: no view strip reserved (PTY 170x52, not 170x51/83x50)" \
  wait_for 10 client_size_is garage/p10e2e-solo/lonely 170x52

# ── 2. Auto-subtitle: pane-title-only change pushed with NO spawn/restart ─
# This is exactly what the part-1 poller fix enables: the poller's
# session-id-set diff alone would never notice a `select-pane -T` — only
# the added normalized-title diff does, and it rides the SAME list-panes
# call (no extra tmux invocation).
tmux select-pane -t "garage/p10e2e-solo/lonely" -T "✳ e2e subtitle"
check "subtitle appears within ~5s via the poller push (no spawn/restart)" \
  wait_for 6 outer_has '✳ e2e subtitle'

# ── 3. d detach: strip appears, notice, frame absent on the solo view ────
focus_ws p10e2e-group
check "focused p10e2e-group (both alpha's and beta's content are gridded)" \
  wait_for 15 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qF alpha-mark-2 && tmux capture-pane -p -t '=$OUTER:' | grep -qF beta-mark-3"
check "two sessions, one (default) view: framed (PTY 83x50)" \
  wait_for 10 client_size_is garage/p10e2e-group/alpha 83x50
# alpha is focused by default (first spawned); detach it.
tmux send-keys -t "=$OUTER:" -l d
check "d detaches: strip names main + alpha" wait_for 10 outer_has ' main  alpha'
check "detach notice shown" outer_has 'detached alpha'
check "solo focused view is frameless (PTY 170x51, not 83x50)" \
  wait_for 10 client_size_is garage/p10e2e-group/alpha 170x51

# ── 3b. D picker from a DETACHED SOLO view — the exact reported repro ────
# shape (scratch daemon + wall, 2 sessions, `d` then `D` while focused on
# the just-detached solo view). The modal must be VISIBLE — title AND row
# text via capture-pane, not merely a state check — and its own current
# view ("alpha", the solo view holding just this session) must be excluded
# from the row list: with only "main" and "alpha" as views, excluding
# "alpha" leaves exactly one real row ("main") ahead of "new group…", so a
# single `j` from the default (row 0, "main") lands on "new group…" — if
# "alpha" were still offered as a confusing self-move row (the entries bug),
# that same `j` would instead land on it.
tmux send-keys -t "=$OUTER:" -l D
check "D opens the picker FROM A DETACHED SOLO VIEW (title visible on screen)" \
  wait_for 10 outer_has 'move to group'
# "new group…" is a row inside the modal's own bordered box and appears
# NOWHERE else on screen (unlike "main", which the view strip above it
# already prints) — an unambiguous proof the row list itself painted, not
# just the title.
check "...its row list is visible too (new group… row painted)" \
  outer_has 'new group…'
tmux send-keys -t "=$OUTER:" -l j       # row 0 "main" -> row 1
tmux send-keys -t "=$OUTER:" Enter
check "alpha's OWN current view is excluded: j landed on new group…, not a self-move" \
  wait_for 10 outer_has 'new group name'
check "...no bogus self-move notice (moved alpha → alpha)" \
  outer_lacks 'moved alpha → alpha'
tmux send-keys -t "=$OUTER:" Escape
check "Esc cancels the picker cleanly" wait_for 10 outer_lacks 'move to group'
check "still detached, unaffected by the picker excursion" outer_has ' main  alpha'

# ── 4. Tab cycles views; grid content follows ────────────────────────────
tmux send-keys -t "=$OUTER:" Tab
check "Tab moves focus to main: beta's content is now gridded" \
  wait_for 10 outer_has 'beta-mark-3'
check "...and alpha's content leaves the grid (it's in the other view)" \
  outer_lacks 'alpha-mark-2'
check "beta's solo-in-main view is also frameless (PTY 170x51)" \
  wait_for 10 client_size_is garage/p10e2e-group/beta 170x51
tmux send-keys -t "=$OUTER:" Tab
check "Tab cycles back to the alpha view (content swaps back)" \
  wait_for 10 outer_has 'alpha-mark-2'
check "PTY size confirms it too (170x51)" \
  wait_for 10 client_size_is garage/p10e2e-group/alpha 170x51

# ── 5. d rejoin: back to one (framed) view ────────────────────────────────
tmux send-keys -t "=$OUTER:" -l d
check "d rejoins: view strip gone (single view again)" \
  wait_for 10 outer_lacks ' main  alpha'
check "rejoined pair is framed again (PTY 83x50 for both)" \
  wait_for 10 sh -c "test \"\$(tmux list-clients -t '=garage/p10e2e-group/alpha' -F '#{client_width}x#{client_height}')\" = 83x50 && test \"\$(tmux list-clients -t '=garage/p10e2e-group/beta' -F '#{client_width}x#{client_height}')\" = 83x50"

# ── 6. D picker FROM THE DEFAULT (joined) VIEW: build a 2-session group ──
# across two moves. alpha is focused (grid order [alpha, beta] after
# reconciliation) and its only view is "main" — its own current view, so
# entries are just ["new group…"] (the confusing self-move row this fix
# drops is never offered in the first place here, since it's the sole view).
tmux send-keys -t "=$OUTER:" -l D
check "D opens the picker (move to group) — visible from the default view too" \
  wait_for 10 outer_has 'move to group'
check "...its row list is visible too (new group… row painted)" \
  outer_has 'new group…'
tmux send-keys -t "=$OUTER:" Enter      # row 0, the only row: "new group…" -> text-input sub-mode
sleep 0.3
tmux send-keys -t "=$OUTER:" -l backend
tmux send-keys -t "=$OUTER:" Enter
check "first move notice: moved alpha → backend" wait_for 10 outer_has 'moved alpha → backend'
check "backend (1 session) is frameless (PTY 170x51)" \
  wait_for 10 client_size_is garage/p10e2e-group/alpha 170x51
# Switch to main (now holds only beta) and move it into the same "backend".
tmux send-keys -t "=$OUTER:" Tab
check "Tab reaches beta in main (its content is gridded)" \
  wait_for 10 outer_has 'beta-mark-3'
tmux send-keys -t "=$OUTER:" -l D
check "D reopens the picker for beta" wait_for 10 outer_has 'move to group'
# beta's current view is "main" (excluded), so entries are just ["backend",
# "new group…"] — row 0 IS "backend" already; no navigation needed. (Before
# this fix, "main" was still offered as a leading self-move row, so reaching
# "backend" needed an extra `j` — its absence here is itself proof the
# exclusion took effect, not just a convenience.)
tmux send-keys -t "=$OUTER:" Enter
check "second move notice: moved beta → backend" wait_for 10 outer_has 'moved beta → backend'
check "backend now has 2 sessions: main vanished (no strip)" \
  wait_for 10 outer_lacks ' main  backend'
check "group frame present (PTY 83x50 for both grouped sessions)" \
  wait_for 10 sh -c "test \"\$(tmux list-clients -t '=garage/p10e2e-group/alpha' -F '#{client_width}x#{client_height}')\" = 83x50 && test \"\$(tmux list-clients -t '=garage/p10e2e-group/beta' -F '#{client_width}x#{client_height}')\" = 83x50"

# ── 7. Cross-view salience: amber dot + a-jump lands engaged ─────────────
focus_ws p10e2e-cross
check "focused p10e2e-cross (both other's and watched's content are gridded)" \
  wait_for 15 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qF other-mark-4 && tmux capture-pane -p -t '=$OUTER:' | grep -qF watched-mark-5"
tmux send-keys -t "=$OUTER:" -l ']'      # cycle onto watched
check "] cycled onto watched" wait_for 10 outer_has '▸ ○ watched'
tmux send-keys -t "=$OUTER:" -l d        # detach it into its own background view
check "watched detached: strip names main + watched" wait_for 10 outer_has ' main  watched'
# Move focus away to another workspace before jumping — a genuine
# cross-workspace, cross-view jump, not merely a same-workspace one.
focus_ws p10e2e-solo
check "moved focus off p10e2e-cross" wait_for 10 outer_has 'keys → garage'
# Built into a variable first, not inlined into the check call: nesting a
# `\"`-escaped JSON literal directly inside a "$(...)" that's itself inside
# another double-quoted argument list is a bash quoting minefield (verified
# to actually triple-evaluate the substitution) — a plain variable sidesteps
# it entirely.
NOTIF_WATCHED=$(printf '{"hook_event_name":"Notification","message":"p10 e2e cross-view","cwd":"%s"}' "$WATCHED_DIR")
check "hook Notification on watched's (unique worktree) dir accepted" \
  test "$(hook "$NOTIF_WATCHED")" = 200
check "top strip reports exactly 1 blocked session" wait_for 10 outer_has '1 blocked'
check "only watched is flagged in the rail (other stays idle)" \
  wait_for 10 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qE '● watched' && ! (tmux capture-pane -p -t '=$OUTER:' | grep -qE '● other')"
tmux send-keys -t "=$OUTER:" -l a
check "a-jump crosses workspace AND view, lands engaged" \
  wait_for 10 outer_has 'keys → p10e2e-cross/watched'
check "view strip shows the amber dot on watched" \
  wait_for 10 outer_has ' main  watched ●'
STOP_WATCHED=$(printf '{"hook_event_name":"Stop","cwd":"%s"}' "$WATCHED_DIR")
check "hook Stop clears needs-input" \
  test "$(hook "$STOP_WATCHED")" = 200
check "amber dot clears after Stop" wait_for 10 outer_lacks ' main  watched ●'
tmux send-keys -t "=$OUTER:" C-g
check "Ctrl+G disengages" wait_for 10 outer_has 'keys → garage'

# ── 8. Persistence: quit, relaunch, groups intact ────────────────────────
tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"
check "wall.json written into the scratch GARAGE_DIR" test -f "$SCRATCH_GARAGE_DIR/wall.json"
check "wall.json records the backend group" \
  grep -qF '"backend"' "$SCRATCH_GARAGE_DIR/wall.json"
check "wall.json records the watched group" \
  grep -qF '"watched"' "$SCRATCH_GARAGE_DIR/wall.json"
check "wall.json did NOT land in the real ~/.garage" \
  sh -c "[ ! -f \"\$HOME/.garage/wall.json\" ] || ! grep -qF p10e2e-cross \"\$HOME/.garage/wall.json\""

rm -f "$WORK/tui-exit"
# The outer tmux session's launch.sh keeps running (its `sleep 120` tail)
# after the TUI process inside it exits, so the tmux session itself is
# still alive — kill it before relaunching under the same name.
tmux kill-session -t "=$OUTER" 2>/dev/null
launch_tui "$OUTER"
check "TUI relaunches against the same scratch daemon" \
  wait_for 15 outer_has 'keys → garage'
focus_ws p10e2e-cross
check "restart preserves the cross-view group (strip shows watched)" \
  wait_for 15 outer_has ' main  watched'
focus_ws p10e2e-group
# `focused_view` (which view is CURRENTLY shown per workspace) is
# deliberately NOT part of wall.json's persisted schema — only session→view
# ASSIGNMENTS are (design.md: `{version, views: {ws: [{name, sessions}]}}`).
# So every workspace's view NAME resets to the default "main" assumption on
# a fresh process. Here "main" is fully vacated (both sessions live in
# "backend") — this used to strand the wall on the empty default with no
# keyboard escape (verification.md "Wall bug found during 4.2"; fixed in
# wall/src/state/store.rs's `refocus_vacated_views`, run on every
# reconcile/load: a focused view with zero members is redirected to the
# workspace's first non-empty view before anything ever renders). So this
# lands directly on "backend" — no spawn workaround, no dead Tab press, no
# phantom "no sessions" hint for a workspace that plainly has sessions.
check "restart with a fully-vacated default: focus lands straight on backend" \
  wait_for 15 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qF alpha-mark-2 && tmux capture-pane -p -t '=$OUTER:' | grep -qF beta-mark-3"
check "...never the phantom empty-workspace hint" outer_lacks 'no sessions in this workspace'
check "...single real view (backend only): no view strip reserved" \
  outer_lacks ' main  backend'
check "...framed, both members restored together (PTY 83x50 — no strip row)" \
  sh -c "test \"\$(tmux list-clients -t '=garage/p10e2e-group/alpha' -F '#{client_width}x#{client_height}')\" = 83x50 && test \"\$(tmux list-clients -t '=garage/p10e2e-group/beta' -F '#{client_width}x#{client_height}')\" = 83x50"
# Only one real view exists, so Tab (cycle_view) must never dead-end NOR
# wander off — it simply has nowhere else to go and stays put.
tmux send-keys -t "=$OUTER:" Tab
sleep 1
check "Tab is a correct no-op with only one real view: still backend, both gridded" \
  sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qF alpha-mark-2 && tmux capture-pane -p -t '=$OUTER:' | grep -qF beta-mark-3"

# ── Quit ───────────────────────────────────────────────────────────────
tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0 (second run)" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"

echo
echo "ALL $CHECKS_RUN CHECKS PASSED — results in $RESULTS (work dir $WORK)"
