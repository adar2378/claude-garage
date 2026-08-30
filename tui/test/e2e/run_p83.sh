#!/usr/bin/env bash
# p8.3 UX additions — e2e harness for the garage TUI.
#
# Covers:
#   A. Rail focus marker: the focused session's rail row carries the `▸`
#      prefix (exactly one on screen), and it follows `]` cycling and a
#      rail session-row click.
#   B. `X` workspace remove: first `X` arms with the strip notice, any
#      other key disarms, `X`-`X` calls registry-only
#      DELETE /api/workspaces/<name> — tmux sessions stay alive and the
#      group resurfaces unregistered; `X` on the now-unregistered group
#      shows the explanatory notice instead of calling the API.
#
# Safety contract (fully scratch — the user's live daemon on 4747 is never
# touched): scratch port (GARAGE_E2E_PORT, default 4795) + scratch
# GARAGE_DIR (so ~/.garage is never read or written); refuses to run when
# the port is taken; only tmux sessions named p83-tui / garage/p83-* are
# created or killed; the destructive X confirm is gated on the strip
# notice naming the expected p83 workspace first.
#
# Exits non-zero on the first failed check; prints PASS/FAIL per check.

set -u -o pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../../.." && pwd)
TUI_BIN=$REPO/tui/dist/garage-tui-darwin-$(node -p 'process.arch' 2>/dev/null || echo arm64)
WORK=${E2E_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/garage-tui-p83.XXXXXX")}
RESULTS=$WORK/results.txt
mkdir -p "$WORK"
: > "$RESULTS"

PORT=${GARAGE_E2E_PORT:-4795}
BASE=http://127.0.0.1:$PORT
SCRATCH_GARAGE_DIR=$WORK/garage-home
mkdir -p "$SCRATCH_GARAGE_DIR"
DAEMON_PID=""
OUTER=p83-tui

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

# Exactly one rail focus marker on screen AND its line names <label>.
marker_on() { # marker_on <label>
  local lines
  lines=$(cap | grep -F '▸')
  [ "$(printf '%s\n' "$lines" | grep -c '▸')" = 1 ] || return 1
  printf '%s\n' "$lines" | grep -qF -- "$1"
}

# Click at 1-based SGR coords: press + release injected as raw hex bytes
# (same transport as run_click_smoke.sh).
click() { # click <x> <y>
  local hex
  hex=$(python3 -c 'import sys
x, y = sys.argv[1], sys.argv[2]
s = f"\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m"
print(" ".join(f"{b:02x}" for b in s.encode()))' "$1" "$2")
  # shellcheck disable=SC2086
  tmux send-keys -t "=$OUTER:" -H $hex
}

# 1-based line number of the first capture line where <needle> appears at a
# character column <= <maxcol> (rail rows also appear in tile titles, so
# the column bound disambiguates). Empty output = not found.
line_of() { # line_of <needle> <maxcol>
  cap | python3 -c 'import sys
needle, maxcol = sys.argv[1], int(sys.argv[2])
for n, line in enumerate(sys.stdin.read().splitlines(), 1):
    i = line.find(needle)
    if 0 <= i < maxcol:
        print(n); break' "$1" "$2"
}

ws_registered() { curl -s -m 2 "$BASE/api/workspaces" | grep -qF '"p83-a"'; }
ws_unregistered() { ! ws_registered; }

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p83-tui|garage/p83-)' \
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
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p83-tui|garage/p83-)' \
  | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done

# ── Daemon up (scratch port + scratch GARAGE_DIR, zsh sessions) ──────────
GARAGE_PORT=$PORT GARAGE_DIR=$SCRATCH_GARAGE_DIR GARAGE_CLAUDE_CMD=/bin/zsh \
  node "$REPO/daemon/src/index.js" >"$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
check "scratch daemon up on :$PORT with scratch GARAGE_DIR" \
  wait_for 10 sh -c "curl -s -m 1 $BASE/api/health | grep -q ok"

# ── Workspace + sessions ─────────────────────────────────────────────────
WS_A=$WORK/p83-a
mkdir -p "$WS_A"
put_ws() { curl -s -o /dev/null -w '%{http_code}' -X PUT "$BASE/api/workspaces" \
  -H 'content-type: application/json' -d "{\"name\":\"$1\",\"dir\":\"$2\"}"; }
spawn() { curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/sessions" \
  -H 'content-type: application/json' -d "{\"workspace\":\"$1\",\"label\":\"$2\"}"; }
check "PUT workspace p83-a"  test "$(put_ws p83-a "$WS_A")" = 200
check "spawn p83-a/main"     test "$(spawn p83-a main)"    = 201
check "spawn p83-a/second"   test "$(spawn p83-a second)"  = 201

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

# tmux is global: the user's real garage/* sessions synthesize foreign
# workspace groups on this wall (same caveat as run_p81.sh) and focus may
# sit on one of them — focus p83-a via its rail number first.
check "p83-a appears in the rail" wait_for 20 sh -c \
  "tmux capture-pane -p -t '=$OUTER:' | grep -qE '[0-9]+ p83-a'"
WS_IDX=$(cap | grep -oE '[0-9]+ p83-a' | head -1 | awk '{print $1}')
note "focusing workspace $WS_IDX p83-a"
tmux send-keys -t "=$OUTER:" -l "$WS_IDX"
check "both tiles attach (main + second in grid titles)" \
  wait_for 20 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qF 'main' && tmux capture-pane -p -t '=$OUTER:' | grep -qF 'second'"

# ── 1. Rail focus marker follows [/] and a click ─────────────────────────
check "focused session main carries the single ▸ rail marker" \
  wait_for 10 marker_on main
tmux send-keys -t "=$OUTER:" -l ']'
check "] moves the marker to second" wait_for 10 marker_on second
tmux send-keys -t "=$OUTER:" -l '['
check "[ moves the marker back to main" wait_for 10 marker_on main
ROW=$(line_of 'second' 28)
[ -n "$ROW" ] || fail "rail row for second not found in capture"
click 6 "$ROW"
check "rail click on second moves the marker to second" \
  wait_for 10 marker_on second
check "rail click did not engage (chip keys → garage)" \
  outer_has 'keys → garage'

# ── 2. X arms, any other key disarms ─────────────────────────────────────
tmux send-keys -t "=$OUTER:" -l X
check "first X arms with the strip notice naming p83-a" \
  wait_for 5 outer_has 'press X again to remove p83-a (sessions keep running)'
tmux send-keys -t "=$OUTER:" -l z
sleep 0.5
check "z disarms (arm notice cleared, workspace still registered)" \
  wait_for 5 sh -c "! tmux capture-pane -p -t '=$OUTER:' | grep -qF 'press X again to remove'"
ws_registered || fail "workspace survived the disarmed X"
pass "workspace survived the disarmed X"

# ── 3. X-X removes registry-only; sessions become unregistered ───────────
tmux send-keys -t "=$OUTER:" -l X
wait_for 5 outer_has 'press X again to remove p83-a' || fail "re-arm before confirm"
tmux send-keys -t "=$OUTER:" -l X
check "X-X removes the registration (gone from /api/workspaces)" \
  wait_for 10 ws_unregistered
check "removal notice shown" \
  wait_for 5 outer_has 'removed workspace p83-a'
check "tmux session main still alive"   tmux has-session -t '=garage/p83-a/main'
check "tmux session second still alive" tmux has-session -t '=garage/p83-a/second'
check "p83-a group resurfaces in the rail (synthesized unregistered)" \
  wait_for 15 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qE '[0-9]+ p83-a'"
check "its sessions are still on the wall (marker still in the rail)" \
  wait_for 10 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qF '▸'"

# ── 4. X on the unregistered group explains instead of removing ──────────
# Focus p83-a again (salience may have reordered the rail after removal).
WS_IDX=$(cap | grep -oE '[0-9]+ p83-a' | head -1 | awk '{print $1}')
tmux send-keys -t "=$OUTER:" -l "$WS_IDX"
sleep 0.5
tmux send-keys -t "=$OUTER:" -l X
check "X on the unregistered group shows the explanatory notice" \
  wait_for 5 outer_has 'already unregistered — sessions live in tmux; x closes them individually'
sleep 0.5
tmux send-keys -t "=$OUTER:" -l X
sleep 1
check "second X on unregistered still removes nothing (sessions alive)" \
  tmux has-session -t '=garage/p83-a/main'

# ── 5. Quit ──────────────────────────────────────────────────────────────
tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"

echo
echo "ALL $CHECKS_RUN CHECKS PASSED — results in $RESULTS (work dir $WORK)"
