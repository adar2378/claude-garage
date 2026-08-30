#!/usr/bin/env bash
# p8-nocterm-tui post-review fix — mouse click smoke for the garage TUI.
#
# Verifies the click contract (spec tui-key-routing "or clicking a tile";
# spec tui-triage "or clicking the strip badge") plus the garage-layer
# typing hint, end to end: SGR mouse press/release sequences are injected
# into the TUI's pane with `tmux send-keys -H` (the same transport the
# wheel smoke used) and assertions read `capture-pane -p`.
#
# Safety contract (p8.1 revision — fully scratch, the user's live daemon on
# 4747 is never touched):
#   - runs its own daemon on a SCRATCH PORT (GARAGE_E2E_PORT, default 4799)
#     with a SCRATCH GARAGE_DIR, so ~/.garage/state.json is never read or
#     written (no backup/restore needed); the TUI is pointed at the scratch
#     daemon via GARAGE_TUI_PORT;
#   - refuses to run when something already listens on the chosen port;
#   - only tmux sessions named e2e-click-tui / garage/e2e-click-* are
#     created or killed; the TUI is stopped by killing ITS scratch tmux
#     session only (never pkill — a user TUI may run the same binary).

set -u -o pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../../.." && pwd)
# Binary under test: GARAGE_TUI_BIN overrides (p9 parity gate points it at
# the Rust binary — same checks); default is the p8 Dart dist binary.
TUI_BIN=${GARAGE_TUI_BIN:-$REPO/tui/dist/garage-tui-darwin-$(node -p 'process.arch' 2>/dev/null || echo arm64)}
WORK=${E2E_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/garage-tui-click.XXXXXX")}
RESULTS=$WORK/results.txt
mkdir -p "$WORK"
: > "$RESULTS"

PORT=${GARAGE_E2E_PORT:-4799}
BASE=http://127.0.0.1:$PORT
SCRATCH_GARAGE_DIR=$WORK/garage-home
mkdir -p "$SCRATCH_GARAGE_DIR"
DAEMON_PID=""
OUTER=e2e-click-tui

CHECKS_RUN=0
pass() { CHECKS_RUN=$((CHECKS_RUN+1)); printf 'PASS %s\n' "$*" | tee -a "$RESULTS"; }
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

# Click at 1-based SGR coords: press + release injected as raw hex bytes.
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
# character column <= <maxcol> (rail rows also appear in tile titles, so the
# column bound disambiguates). Empty output = not found.
line_of() { # line_of <needle> <maxcol>
  cap | python3 -c 'import sys
needle, maxcol = sys.argv[1], int(sys.argv[2])
for n, line in enumerate(sys.stdin.read().splitlines(), 1):
    i = line.find(needle)
    if 0 <= i < maxcol:
        print(n); break' "$1" "$2"
}

# 1-based character column of <needle> in capture line <lineno> (all strip
# glyphs are width-1, so char index == display column).
col_of() { # col_of <needle> <lineno>
  cap | python3 -c 'import sys
needle, lineno = sys.argv[1], int(sys.argv[2])
lines = sys.stdin.read().splitlines()
line = lines[lineno - 1] if lineno <= len(lines) else ""
i = line.find(needle)
print(i + 1 if i >= 0 else "")' "$1" "$2"
}

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(e2e-click-tui|garage/e2e-click-)' \
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
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(e2e-click-tui|garage/e2e-click-)' \
  | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done

# ── Daemon up (scratch port + scratch GARAGE_DIR, zsh sessions) ──────────
GARAGE_PORT=$PORT GARAGE_DIR=$SCRATCH_GARAGE_DIR GARAGE_CLAUDE_CMD=/bin/zsh \
  node "$REPO/daemon/src/index.js" >"$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
check "scratch daemon up" wait_for 10 sh -c "curl -s -m 1 $BASE/api/health | grep -q ok"
TOKEN=$(curl -s "$BASE/api/hooks/snippet" | python3 -c 'import json,sys;print(json.load(sys.stdin)["hooks"]["Notification"][0]["hooks"][0]["url"].split("token=")[1])')

# ── Scratch workspaces + sessions ────────────────────────────────────────
WS_A=$WORK/ws-e2e-click-a
WS_B=$WORK/ws-e2e-click-b
mkdir -p "$WS_A" "$WS_B"
put_ws() { curl -s -o /dev/null -w '%{http_code}' -X PUT "$BASE/api/workspaces" \
  -H 'content-type: application/json' -d "{\"name\":\"$1\",\"dir\":\"$2\"}"; }
spawn() { curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/sessions" \
  -H 'content-type: application/json' -d "{\"workspace\":\"$1\",\"label\":\"$2\"}"; }
check "PUT workspace e2e-click-a" test "$(put_ws e2e-click-a "$WS_A")" = 200
check "PUT workspace e2e-click-b" test "$(put_ws e2e-click-b "$WS_B")" = 200
check "spawn e2e-click-a/main"    test "$(spawn e2e-click-a main)"   = 201
check "spawn e2e-click-a/second"  test "$(spawn e2e-click-a second)" = 201
check "spawn e2e-click-b/bmark"   test "$(spawn e2e-click-b bmark)"  = 201

# Distinctive pane content so grid assertions can tell tiles apart.
tmux send-keys -t "=garage/e2e-click-a/main:"   -l 'echo alpha-mark-111'
tmux send-keys -t "=garage/e2e-click-a/main:"   Enter
tmux send-keys -t "=garage/e2e-click-b/bmark:"  -l 'echo beta-mark-777'
tmux send-keys -t "=garage/e2e-click-b/bmark:"  Enter

# ── Launch the TUI in a scratch outer tmux session (200x55) ──────────────
cat > "$WORK/launch.sh" <<EOF
#!/bin/sh
"$TUI_BIN"
echo \$? > "$WORK/tui-exit"
sleep 120
EOF
chmod +x "$WORK/launch.sh"
tmux new-session -d -x 200 -y 55 -e GARAGE_TUI_PORT="$PORT" -s "$OUTER" "$WORK/launch.sh"
check "TUI renders rail (e2e-click-a visible)" wait_for 15 outer_has 'e2e-click-a'
check "alpha marker streams into a tile"       wait_for 15 outer_has 'alpha-mark-111'

# Grid geometry at 200x55: rail 28 wide, strip 1 row → grid 172x54 at
# x=28..199. Two tiles → cells local [0..85] and [86..171].
# Tile 1 center ≈ local (40,27) → SGR (69,28); tile 2 ≈ (130,27) → (159,28).

# ── 1. Click tile 2: focus AND engage ────────────────────────────────────
click 159 28
check "click tile 2 engages it (chip keys → e2e-click-a/second)" \
  wait_for 10 outer_has 'keys → e2e-click-a/second'

# ── 2. Click the engaged tile again: nothing extra ───────────────────────
click 159 28
sleep 1
check "click on engaged tile is a no-op (chip unchanged)" \
  outer_has 'keys → e2e-click-a/second'

# ── 3. Click tile 1 while engaged: engagement migrates ───────────────────
click 69 28
check "click other tile migrates engagement (chip keys → e2e-click-a/main)" \
  wait_for 10 outer_has 'keys → e2e-click-a/main'

# ── 4. Rail session row click: disengage + focus (no engage) ─────────────
ROW=$(line_of 'bmark' 28)
[ -n "$ROW" ] || fail "rail row for bmark not found in capture"
click 6 "$ROW"
check "rail session click focuses it (beta marker swaps into grid)" \
  wait_for 15 outer_has 'beta-mark-777'
check "rail session click does not engage (chip keys → garage)" \
  wait_for 5 outer_has 'keys → garage'

# ── 5. Rail workspace header click: focus the workspace ──────────────────
ROW=$(line_of 'e2e-click-a' 28)
[ -n "$ROW" ] || fail "rail header for e2e-click-a not found in capture"
click 6 "$ROW"
check "rail header click focuses workspace (alpha marker back in grid)" \
  wait_for 15 outer_has 'alpha-mark-111'
check "still garage after header click" outer_has 'keys → garage'

# ── 6. Blocked badge click opens the triage queue ────────────────────────
hook() { curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/hooks/claude?token=$TOKEN" \
  -H 'content-type: application/json' -d "$1"; }
NOTIF='{"hook_event_name":"Notification","message":"click e2e question","cwd":"'"$WS_B"'"}'
check "hook Notification accepted (200)" test "$(hook "$NOTIF")" = 200
check "strip shows blocked badge" wait_for 10 outer_has 'blocked'
BCOL=$(col_of 'blocked' 55)
[ -n "$BCOL" ] || fail "badge column not found in strip line 55"
click "$BCOL" 55
check "badge click opens triage queue (chip keys → queue)" \
  wait_for 10 outer_has 'keys → queue'
check "queue row shows daemon message" wait_for 5 outer_has 'click e2e question'

# ── 7. Queue row click: select + jump-engage ─────────────────────────────
QROW=$(line_of 'click e2e question' 200)
[ -n "$QROW" ] || fail "queue row line not found"
click 75 "$QROW"
check "queue row click jump-engages the blocked session" \
  wait_for 10 outer_has 'keys → e2e-click-b/bmark'
tmux send-keys -t "=$OUTER:" C-g
wait_for 10 outer_has 'keys → garage' || fail "Ctrl+G disengage after queue jump"

# ── 8. Click outside the modal closes the queue ──────────────────────────
BCOL=$(col_of 'blocked' 55)
[ -n "$BCOL" ] || fail "badge column not found for reopen"
click "$BCOL" 55
wait_for 10 outer_has 'keys → queue' || fail "badge reopen before outside-click test"
click 4 4
check "click outside modal closes the queue (chip keys → garage)" \
  wait_for 10 outer_has 'keys → garage'

# ── 9. Help overlay: any click closes ────────────────────────────────────
tmux send-keys -t "=$OUTER:" -l '?'
wait_for 10 outer_has 'keys → help' || fail "? opens help before click test"
click 100 20
check "click closes the help overlay" wait_for 10 outer_has 'keys → garage'

# ── 10. Garage-layer typing hint appears and clears ──────────────────────
# `z` — an UNBOUND printable (p8.1 bound `x` to the armed close).
tmux send-keys -t "=$OUTER:" -l z
check "typing in garage shows the hint notice" \
  wait_for 5 outer_has 'enter engages the focused terminal'
check "hint clears after ~2.5s" \
  wait_for 8 outer_lacks 'enter engages the focused terminal'

# ── Clear needs-input, quit ──────────────────────────────────────────────
STOP='{"hook_event_name":"Stop","cwd":"'"$WS_B"'"}'
check "hook Stop accepted (200)" test "$(hook "$STOP")" = 200
tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"

echo
echo "ALL $CHECKS_RUN CHECKS PASSED — results in $RESULTS"
