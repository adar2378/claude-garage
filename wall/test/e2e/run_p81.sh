#!/usr/bin/env bash
# p8.1 basic-functionality wave — e2e harness for the garage TUI.
#
# Covers: tile-size → PTY propagation (tmux client_width/height must match
# the tile inner size, and follow maximize), `m` maximize toggle, Enter/R
# restore round-trips, `x`-`x` armed close (live + restorable meta,
# worktree-kept notice), the `w` add-workspace overlay, and empty states.
#
# Safety contract (fully scratch — the user's live daemon on 4747 is never
# touched):
#   - runs its own daemon on a SCRATCH PORT (GARAGE_E2E_PORT, default 4798)
#     with a SCRATCH GARAGE_DIR, so ~/.garage is never read or written;
#     the TUI is pointed at it via GARAGE_TUI_PORT;
#   - refuses to run when something already listens on the chosen port;
#   - only tmux sessions named p81-tui / garage/p81-* are created or
#     killed; keys are only ever sent to the TUI while a p81 workspace is
#     focused, and every destructive `x` confirm is gated on the strip
#     notice naming the expected p81 label first.
#
# Exits non-zero on the first failed check; prints PASS/FAIL per check.

set -u -o pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../../.." && pwd)
# Binary under test: GARAGE_TUI_BIN overrides (p9 parity gate points it at
# the Rust binary — same checks); default is the Rust dist binary (wall/dist).
TUI_BIN=${GARAGE_TUI_BIN:-$REPO/wall/dist/garage-wall-darwin-$(node -p 'process.arch' 2>/dev/null || echo arm64)}
WORK=${E2E_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/garage-wall-p81.XXXXXX")}
RESULTS=$WORK/results.txt
mkdir -p "$WORK"
: > "$RESULTS"

PORT=${GARAGE_E2E_PORT:-4798}
BASE=http://127.0.0.1:$PORT
SCRATCH_GARAGE_DIR=$WORK/garage-home
STATE=$SCRATCH_GARAGE_DIR/state.json
mkdir -p "$SCRATCH_GARAGE_DIR"
DAEMON_PID=""
OUTER=p81-tui

CHECKS_RUN=0
note() { printf '     %s\n' "$*"; }
pass() { CHECKS_RUN=$((CHECKS_RUN+1)); printf 'PASS %s\n' "$*" | tee -a "$RESULTS"; }
skip() { printf 'SKIP %s\n' "$*" | tee -a "$RESULTS"; }
fail() {
  CHECKS_RUN=$((CHECKS_RUN+1)); printf 'FAIL %s\n' "$*" | tee -a "$RESULTS"
  tmux capture-pane -p -t "=$OUTER:" > "$WORK/fail-capture.txt" 2>/dev/null
  curl -s -m 2 "$BASE/api/sessions" > "$WORK/fail-sessions.json" 2>/dev/null
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

# The tile attach client's size for a garage session ("<w>x<h>", empty if
# no client attached).
client_size() { tmux list-clients -t "=$1" -F '#{client_width}x#{client_height}' 2>/dev/null | head -1; }
client_size_is() { [ "$(client_size "$1")" = "$2" ]; }

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p81-tui|garage/p81-)' \
    | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done
  if [ -n "$DAEMON_PID" ]; then kill "$DAEMON_PID" 2>/dev/null; wait "$DAEMON_PID" 2>/dev/null; fi
  # Fully scratch: state lived in $SCRATCH_GARAGE_DIR; ~/.garage untouched.
  exit "$rc"
}
trap cleanup EXIT INT TERM

# ── Preflight ────────────────────────────────────────────────────────────
[ -x "$TUI_BIN" ] || { echo "TUI binary missing: $TUI_BIN"; exit 2; }
for tool in tmux node curl python3; do
  command -v "$tool" >/dev/null || { echo "missing tool: $tool"; exit 2; }
done
if curl -s -m 2 "$BASE/api/health" | grep -q ok; then
  echo "refusing to run: something is already listening on $BASE (set GARAGE_E2E_PORT)"; exit 2
fi
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p81-tui|garage/p81-)' \
  | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done

# tmux is global: garage/* sessions from the user's REAL daemon appear in
# the scratch daemon's listing as synthesized workspace groups. They are
# never touched; the zero-workspace empty-state check is skipped when any
# exist (the wall then isn't empty).
FOREIGN_GARAGE=$(tmux ls -F '#{session_name}' 2>/dev/null | grep -c '^garage/' || true)

# ── Daemon up (scratch port + scratch GARAGE_DIR, zsh sessions) ──────────
GARAGE_PORT=$PORT GARAGE_DIR=$SCRATCH_GARAGE_DIR GARAGE_CLAUDE_CMD=/bin/zsh \
  node "$REPO/daemon/src/index.js" >"$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
check "scratch daemon up on :$PORT with scratch GARAGE_DIR" \
  wait_for 10 sh -c "curl -s -m 1 $BASE/api/health | grep -q ok"

# The p81-a workspace dir: a real git repo so a worktree session can spawn.
WS_A=$WORK/p81-a
mkdir -p "$WS_A"
git -C "$WS_A" init -q
git -C "$WS_A" -c user.email=p81@e2e -c user.name=p81 commit -q --allow-empty -m init

# ── Launch the TUI (200x55) against the scratch daemon ───────────────────
cat > "$WORK/launch.sh" <<EOF
#!/bin/sh
"$TUI_BIN"
echo \$? > "$WORK/tui-exit"
sleep 120
EOF
chmod +x "$WORK/launch.sh"
tmux new-session -d -x 200 -y 55 -e GARAGE_TUI_PORT="$PORT" -s "$OUTER" "$WORK/launch.sh"
check "TUI launches against the scratch daemon (strip chip renders)" \
  wait_for 15 outer_has 'keys → garage'

# ── 1. Empty states ──────────────────────────────────────────────────────
if [ "$FOREIGN_GARAGE" -eq 0 ]; then
  check "zero workspaces: onboarding panel points at w" \
    wait_for 10 outer_has 'no workspaces yet — press w to add one'
else
  skip "zero-workspace onboarding panel ($FOREIGN_GARAGE foreign garage/* tmux sessions synthesize groups)"
fi

# ── 2. w add-workspace overlay ───────────────────────────────────────────
tmux send-keys -t "=$OUTER:" -l w
check "w opens the add-workspace overlay (chip keys → add workspace)" \
  wait_for 10 outer_has 'keys → add workspace'
check "overlay renders the path field affordances" \
  wait_for 5 outer_has 'Enter add · ^O browse · Esc cancel'
# Esc cancels…
tmux send-keys -t "=$OUTER:" Escape
check "Esc cancels the overlay (chip back to keys → garage)" \
  wait_for 10 outer_has 'keys → garage'
# …and a bad path keeps it open with an inline error.
tmux send-keys -t "=$OUTER:" -l w
wait_for 10 outer_has 'keys → add workspace' || fail "w reopen for bad-path test"
tmux send-keys -t "=$OUTER:" -l "$WORK/does-not-exist"
tmux send-keys -t "=$OUTER:" Enter
check "nonexistent dir shows an inline error, overlay stays open" \
  wait_for 10 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qF 'no such directory' && tmux capture-pane -p -t '=$OUTER:' | grep -qF 'keys → add workspace'"
# Clear the field (one backspace per typed char) and submit the real path.
BAD_LEN=$(printf '%s' "$WORK/does-not-exist" | wc -c | tr -d ' ')
for _ in $(seq 1 "$BAD_LEN"); do tmux send-keys -t "=$OUTER:" BSpace; done
tmux send-keys -t "=$OUTER:" -l "$WS_A"
tmux send-keys -t "=$OUTER:" Enter
check "valid path registers the workspace (derived name p81-a in strip)" \
  wait_for 15 outer_has ':p81-a'
check "new workspace is focused and shows the empty-workspace spawn hint" \
  wait_for 10 outer_has 'press n for a session, N for a worktree session'

# ── 3. Sessions + tile-size → PTY propagation ────────────────────────────
spawn() { curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/sessions" \
  -H 'content-type: application/json' -d "{\"workspace\":\"$1\",\"label\":\"$2\"$3}"; }
check "spawn p81-a/main"   test "$(spawn p81-a main '')"   = 201
check "spawn p81-a/second" test "$(spawn p81-a second '')" = 201
check "both tiles attach (main + second in grid titles)" \
  wait_for 20 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qF 'main' && tmux capture-pane -p -t '=$OUTER:' | grep -qF 'second'"

# Geometry at 200x55: rail 28, strip 1 → grid 172x54. p10 (spec tui-views
# "View strip and group frame"): a view with 2+ sessions ALWAYS gets the
# neutral group frame around the grid area, even the lone default view of a
# workspace that just happens to have two sessions and no manual grouping —
# so this p81-a/{main,second} pair is framed too. Frame insets grid_area by
# 1 cell/side → grid 170x52; two tiles → cells ~85x52 each → inner (borders
# off) 83x50. Maximized → fills the (already framed-inset) grid 170x52 →
# inner 168x50. Verified against a live run after the p10 change landed;
# these were 84x52 / 170x52 / 84x52 before framing existed.
check "PTY size matches tile inner size for main (83x50, not 80x24)" \
  wait_for 15 client_size_is garage/p81-a/main 83x50
check "PTY size matches tile inner size for second (83x50)" \
  wait_for 15 client_size_is garage/p81-a/second 83x50

# ── 4. m maximize toggles tile AND PTY size ──────────────────────────────
tmux send-keys -t "=$OUTER:" -l m
check "m maximizes the focused tile (PTY grows to 168x50)" \
  wait_for 15 client_size_is garage/p81-a/main 168x50
check "obscured sibling keeps its grid-cell PTY size (83x50)" \
  client_size_is garage/p81-a/second 83x50
tmux send-keys -t "=$OUTER:" -l m
check "m again restores the grid (PTY back to 83x50)" \
  wait_for 15 client_size_is garage/p81-a/main 83x50

# ── 5. Restore round-trip (Enter on a restorable tile) ───────────────────
# GARAGE_CLAUDE_CMD=/bin/zsh means no poller-observed claudeSessionId, so
# inject resume metadata directly into the SCRATCH state.json (the daemon
# re-reads it per request). `zsh --resume <id>` exits, so the daemon's
# fallback spawns a fresh session — the round-trip still lands live.
add_meta() { # add_meta <label>
  python3 - "$STATE" "$1" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))
label = sys.argv[2]
state.setdefault("sessions", {})[f"garage/p81-a/{label}"] = {
    "claudeSessionId": f"p81-fake-{label}",
    "workspace": "p81-a",
    "label": label,
    "worktree": None,
}
json.dump(state, open(sys.argv[1], "w"), indent=2)
PY
}
add_meta second
tmux kill-session -t "=garage/p81-a/second"
check "killed session shows the restorable placeholder" \
  wait_for 25 outer_has 'press Enter (or click) to restore'
# Focus the restorable tile (grid order [main, second]; main is focused).
tmux send-keys -t "=$OUTER:" -l ']'
tmux send-keys -t "=$OUTER:" Enter
check "Enter on the restorable tile shows the restoring placeholder" \
  wait_for 10 outer_has 'restoring'
check "restore round-trip: tmux session is live again" \
  wait_for 25 tmux has-session -t '=garage/p81-a/second'
check "restorable placeholder gone after restore" \
  wait_for 20 outer_lacks 'press Enter (or click) to restore'

# ── 6. R restores every restorable in the workspace ──────────────────────
add_meta second
add_meta ghost   # never had a tmux session — pure restorable meta
tmux kill-session -t "=garage/p81-a/second"
check "two restorable placeholders (second killed + ghost meta)" \
  wait_for 25 sh -c "curl -s $BASE/api/sessions | python3 -c 'import json,sys; s=json.load(sys.stdin); print(sum(1 for x in s if x.get(\"restorable\")))' | grep -qx 2"
wait_for 25 outer_has 'press Enter (or click) to restore' || fail "restorable placeholder before R"
tmux send-keys -t "=$OUTER:" -l R
check "R restores second (tmux session live)" \
  wait_for 30 tmux has-session -t '=garage/p81-a/second'
check "R restores ghost (tmux session live)" \
  wait_for 30 tmux has-session -t '=garage/p81-a/ghost'

# ── 7. x-x armed close of a live session (with disarm) ───────────────────
# Focus ghost deterministically: it joined the grid last ([main, second,
# ghost]); reconciliation kept main/second focused-ordering, so cycle until
# the arm notice names ghost — gated below before any confirm.
wait_for 20 outer_lacks 'restoring' || fail "restores settled before close test"
tmux send-keys -t "=$OUTER:" -l ']'
sleep 0.3
tmux send-keys -t "=$OUTER:" -l x
if ! wait_for 5 outer_has 'press x again to close ghost'; then
  # focus landed elsewhere in the cycle — advance once more and re-arm
  tmux send-keys -t "=$OUTER:" -l ']'
  sleep 0.3
  tmux send-keys -t "=$OUTER:" -l x
fi
check "first x arms with the strip notice naming ghost" \
  wait_for 5 outer_has 'press x again to close ghost'
# Any other key disarms: z (unbound) → the next x must ARM again, not close.
tmux send-keys -t "=$OUTER:" -l z
sleep 0.5
tmux send-keys -t "=$OUTER:" -l x
sleep 1
check "disarm works: ghost still alive after x·z·x" \
  tmux has-session -t '=garage/p81-a/ghost'
wait_for 5 outer_has 'press x again to close ghost' || fail "re-arm notice after disarm"
tmux send-keys -t "=$OUTER:" -l x
check "second x closes the live session (tmux session gone)" \
  wait_for 15 sh -c "! tmux has-session -t '=garage/p81-a/ghost' 2>/dev/null"
check "close notice shown" wait_for 5 outer_has 'closed ghost'

# ── 8. x-x on a restorable drops only the stored meta (?meta=1) ──────────
add_meta stale   # restorable placeholder with no tmux session ever
check "stale meta listed as restorable" \
  wait_for 20 sh -c "curl -s $BASE/api/sessions | grep -qF 'p81-a/stale'"
wait_for 20 outer_has 'press Enter (or click) to restore' || fail "stale placeholder renders"
# Focus it (grid [main, second, stale] after ghost left; cycle to it).
tmux send-keys -t "=$OUTER:" -l ']'
sleep 0.3
tmux send-keys -t "=$OUTER:" -l x
if ! wait_for 5 outer_has 'press x again to close stale'; then
  tmux send-keys -t "=$OUTER:" -l ']'
  sleep 0.3
  tmux send-keys -t "=$OUTER:" -l x
  if ! wait_for 5 outer_has 'press x again to close stale'; then
    tmux send-keys -t "=$OUTER:" -l ']'
    sleep 0.3
    tmux send-keys -t "=$OUTER:" -l x
  fi
fi
check "x arms on the restorable placeholder" \
  wait_for 5 outer_has 'press x again to close stale'
tmux send-keys -t "=$OUTER:" -l x
check "x-x on a restorable drops the meta (gone from /api/sessions)" \
  wait_for 15 sh -c "! curl -s $BASE/api/sessions | grep -qF 'p81-a/stale'"

# ── 9. Worktree session close keeps the worktree with a notice ───────────
check "spawn worktree session p81-a/wt" test "$(spawn p81-a wt ',"worktree":true')" = 201
check "wt tile appears" wait_for 20 outer_has 'wt'
WT_DIR=$SCRATCH_GARAGE_DIR/worktrees/p81-a/wt
[ -d "$WT_DIR" ] || fail "worktree dir exists under the scratch GARAGE_DIR"
tmux send-keys -t "=$OUTER:" -l ']'
sleep 0.3
tmux send-keys -t "=$OUTER:" -l x
if ! wait_for 5 outer_has 'press x again to close wt'; then
  tmux send-keys -t "=$OUTER:" -l ']'
  sleep 0.3
  tmux send-keys -t "=$OUTER:" -l x
fi
check "x arms on the worktree session" \
  wait_for 5 outer_has 'press x again to close wt'
tmux send-keys -t "=$OUTER:" -l x
check "worktree-kept notice names the branch and the web wall" \
  wait_for 15 outer_has 'worktree kept:'
check "worktree dir survives the close (v1 keep policy)" test -d "$WT_DIR"
check "wt tmux session gone" \
  wait_for 10 sh -c "! tmux has-session -t '=garage/p81-a/wt' 2>/dev/null"

# ── 10. Quit ─────────────────────────────────────────────────────────────
tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"

echo
echo "ALL $CHECKS_RUN CHECKS PASSED — results in $RESULTS (work dir $WORK)"
