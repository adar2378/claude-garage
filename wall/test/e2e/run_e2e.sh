#!/usr/bin/env bash
# p8-nocterm-tui task 10.1 + 10.2 — tmux-driven e2e harness for the garage TUI.
#
# Pattern (spike + p8 design.md): the compiled TUI runs inside a detached
# scratch tmux session; keys are injected with `tmux send-keys` (raw hex for
# escape sequences) and assertions read `tmux capture-pane -p` of the OUTER
# pane (TUI chrome) or the INNER garage sessions (byte-exactness).
#
# Safety contract (fully scratch — the user's live daemon on 4747 is never
# touched):
#   - runs its own daemon on a SCRATCH PORT (GARAGE_E2E_PORT, default 4794)
#     with a SCRATCH GARAGE_DIR, so ~/.garage is never read or written
#     (state.json restore plumbing retained for a pre-existing scratch state).
#   - Only tmux sessions named e2e-* / garage/e2e-*/... are created or killed.
#   - The daemon is started by this script (refuses to run against an already
#     running daemon on the scratch port) with GARAGE_CLAUDE_CMD=/bin/zsh and
#     killed on exit.
#
# Exits non-zero on the first failed check; prints PASS/FAIL per check.

set -u -o pipefail

# ── Paths ────────────────────────────────────────────────────────────────
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../../.." && pwd)
# Binary under test: GARAGE_TUI_BIN overrides (p9 parity gate points it at
# the Rust binary — same checks); default is the Rust dist binary (wall/dist).
TUI_BIN=${GARAGE_TUI_BIN:-$REPO/wall/dist/garage-wall-darwin-$(node -p 'process.arch' 2>/dev/null || echo arm64)}
KEYECHO=$REPO/wall/test/e2e/tools/keyecho.sh
STRESS=$REPO/wall/test/e2e/tools/stress.sh
WORK=${E2E_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/garage-wall-e2e.XXXXXX")}
KEYLOG=$WORK/keylog.txt
RESULTS=$WORK/results.txt
mkdir -p "$WORK"
: > "$RESULTS"

# Scratch port + scratch GARAGE_DIR (p9 parity gate): the user's live daemon
# on 4747 and ~/.garage are never touched. Same pattern as run_p81–p84.
PORT=${GARAGE_E2E_PORT:-4794}
SCRATCH_GARAGE_DIR=$WORK/garage-home
STATE=$SCRATCH_GARAGE_DIR/state.json
mkdir -p "$SCRATCH_GARAGE_DIR"

BASE=http://127.0.0.1:$PORT
DAEMON_PID=""

# ── Check plumbing ───────────────────────────────────────────────────────
CHECKS_RUN=0
note()  { printf '     %s\n' "$*"; }
pass()  { CHECKS_RUN=$((CHECKS_RUN+1)); printf 'PASS %s\n' "$*" | tee -a "$RESULTS"; }
fail()  {
  CHECKS_RUN=$((CHECKS_RUN+1)); printf 'FAIL %s\n' "$*" | tee -a "$RESULTS"
  # Diagnostics for triage: outer TUI frame + daemon session listing.
  tmux capture-pane -p -t "=${OUTER:-e2e-tui}:" > "$WORK/fail-capture.txt" 2>/dev/null
  curl -s -m 2 "$BASE/api/sessions" > "$WORK/fail-sessions.json" 2>/dev/null
  exit 1
}

check() { # check <description> <command...>
  local desc=$1; shift
  if "$@"; then pass "$desc"; else fail "$desc"; fi
}

# Poll until <command> succeeds or <timeout-seconds> elapse.
wait_for() { # wait_for <timeout> <command...>
  local timeout=$1 t0; shift
  t0=$(date +%s)
  while true; do
    if "$@"; then return 0; fi
    [ $(( $(date +%s) - t0 )) -ge "$timeout" ] && return 1
    sleep 0.2
  done
}

outer_has()  { tmux capture-pane -p -t "=$OUTER:" 2>/dev/null | grep -qF -- "$1"; }
outer_lacks(){ ! outer_has "$1"; }
inner_has()  { tmux capture-pane -p -t "=$1:" 2>/dev/null | grep -qF -- "$2"; }

now_us() { perl -MTime::HiRes=time -e 'printf "%d", time()*1000000'; }

# ── Cleanup (runs on every exit path) ────────────────────────────────────
cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  # Kill only sessions this harness created.
  tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(e2e-tui|garage/e2e-)' \
    | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done
  pkill -f "$TUI_BIN" 2>/dev/null
  if [ -n "$DAEMON_PID" ]; then kill "$DAEMON_PID" 2>/dev/null; wait "$DAEMON_PID" 2>/dev/null; fi
  # Restore state.json byte-exact only after the daemon can no longer write.
  if [ -f "$WORK/state.json.orig" ]; then
    cp "$WORK/state.json.orig" "$STATE"
    if cmp -s "$WORK/state.json.orig" "$STATE"; then
      echo "state.json restored byte-exact" | tee -a "$RESULTS"
    else
      echo "ERROR: state.json restore mismatch" | tee -a "$RESULTS"; rc=1
    fi
  fi
  exit "$rc"
}
trap cleanup EXIT INT TERM

# ── Preflight ────────────────────────────────────────────────────────────
[ -x "$TUI_BIN" ] || { echo "TUI binary missing: $TUI_BIN"; exit 2; }
[ -x "$KEYECHO" ] || { echo "keyecho.sh missing: $KEYECHO"; exit 2; }
for tool in tmux node curl python3 perl; do
  command -v "$tool" >/dev/null || { echo "missing tool: $tool"; exit 2; }
done
if curl -s -m 2 "$BASE/api/health" | grep -q ok; then
  echo "refusing to run: a daemon is already listening on $BASE (set GARAGE_E2E_PORT)"; exit 2
fi
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(e2e-tui|garage/e2e-)' \
  | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done

# ── State swap: scratch registry, original hookToken ─────────────────────
if [ -f "$STATE" ]; then cp "$STATE" "$WORK/state.json.orig"; fi
python3 - "$WORK/state.json.orig" "$STATE" <<'PY'
import json, sys, os
orig = {}
if os.path.exists(sys.argv[1]):
    orig = json.load(open(sys.argv[1]))
scratch = {"workspaces": {}, "hookToken": orig.get("hookToken", ""), "sessions": {}}
json.dump(scratch, open(sys.argv[2], "w"), indent=2)
PY
TOKEN=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("hookToken",""))' "$WORK/state.json.orig" 2>/dev/null || true)

# ── Daemon up (scratch port + scratch GARAGE_DIR) ────────────────────────
GARAGE_PORT=$PORT GARAGE_DIR=$SCRATCH_GARAGE_DIR GARAGE_CLAUDE_CMD=/bin/zsh \
  node "$REPO/daemon/src/index.js" >"$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
check "daemon up (GET /api/health ok)" \
  wait_for 10 sh -c "curl -s -m 1 $BASE/api/health | grep -q ok"

if [ -z "$TOKEN" ]; then
  TOKEN=$(curl -s "$BASE/api/hooks/snippet" | python3 -c 'import json,sys;print(json.load(sys.stdin)["hooks"]["Notification"][0]["hooks"][0]["url"].split("token=")[1])')
fi

# ── Scratch workspaces + sessions ────────────────────────────────────────
WS_ALPHA=$WORK/ws-e2e-alpha
WS_BETA=$WORK/ws-e2e-beta
mkdir -p "$WS_ALPHA" "$WS_BETA"

put_ws() { curl -s -o /dev/null -w '%{http_code}' -X PUT "$BASE/api/workspaces" \
  -H 'content-type: application/json' -d "{\"name\":\"$1\",\"dir\":\"$2\"}"; }
PW1=$(put_ws e2e-alpha "$WS_ALPHA"); check "PUT /api/workspaces e2e-alpha" test "$PW1" = 200
PW2=$(put_ws e2e-beta  "$WS_BETA");  check "PUT /api/workspaces e2e-beta"  test "$PW2" = 200

spawn() { curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/sessions" \
  -H 'content-type: application/json' -d "{\"workspace\":\"$1\",\"label\":\"$2\"}"; }
SP1=$(spawn e2e-alpha main);   check "POST /api/sessions e2e-alpha/main"   test "$SP1" = 201
SP2=$(spawn e2e-beta keyecho); check "POST /api/sessions e2e-beta/keyecho" test "$SP2" = 201

# Turn the beta session into a byte-echo target (spike tool, exec'd so the
# pane root process becomes keyecho itself).
tmux send-keys -t "=garage/e2e-beta/keyecho:" -l "exec $KEYECHO"
tmux send-keys -t "=garage/e2e-beta/keyecho:" Enter
check "keyecho running in inner e2e-beta session" \
  wait_for 10 inner_has 'garage/e2e-beta/keyecho' 'keyecho ready'

# Foreign-safety guard: if any REAL (non-e2e) session reports needs-input in
# this daemon's in-memory store, clear it with a Stop hook (store-only write;
# the real tmux session is untouched) so triage jumps can never land on it.
clear_foreign_blocked() {
  curl -s "$BASE/api/sessions" | python3 -c '
import json, sys
for s in json.load(sys.stdin):
    if s.get("status") == "needs-input" and not s["id"].startswith("garage/e2e-"):
        print(s.get("dir") or "")' | while IFS= read -r d; do
    [ -n "$d" ] && curl -s -o /dev/null -X POST "$BASE/api/hooks/claude?token=$TOKEN" \
      -H 'content-type: application/json' \
      -d "{\"hook_event_name\":\"Stop\",\"cwd\":\"$d\"}"
    echo "  (cleared foreign needs-input at $d in scratch daemon store)"
  done
}
clear_foreign_blocked

# ── Launch the TUI in a scratch outer tmux session ───────────────────────
launch_tui() { # launch_tui <session-name> <cols> <rows> <tag>
  local name=$1 cols=$2 rows=$3 tag=$4
  cat > "$WORK/launch-$tag.sh" <<EOF
#!/bin/sh
"$TUI_BIN"
echo \$? > "$WORK/tui-exit-$tag"
stty -a < /dev/tty > "$WORK/tui-stty-$tag" 2>&1
sleep 120
EOF
  chmod +x "$WORK/launch-$tag.sh"
  tmux new-session -d -x "$cols" -y "$rows" \
    -e GARAGE_TUI_KEYLOG="$KEYLOG" -e GARAGE_TUI_PORT="$PORT" \
    -s "$name" "$WORK/launch-$tag.sh"
}

OUTER=e2e-tui
launch_tui "$OUTER" 200 55 s200
check "TUI renders rail with e2e-alpha workspace (200x55)" \
  wait_for 15 outer_has 'e2e-alpha'
check "strip shows workspace tabs 1:e2e-alpha 2:e2e-beta" \
  wait_for 10 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qF '1:e2e-alpha' && tmux capture-pane -p -t '=$OUTER:' | grep -qF '2:e2e-beta'"

# ── Digit focus ──────────────────────────────────────────────────────────
tmux send-keys -t "=$OUTER:" -l 2
check "digit 2 focuses e2e-beta (keyecho tile content in grid)" \
  wait_for 15 outer_has 'keyecho ready'
tmux send-keys -t "=$OUTER:" -l 1
check "digit 1 focuses e2e-alpha (keyecho tile leaves grid)" \
  wait_for 15 outer_lacks 'keyecho ready'

# ── Engage + chip ────────────────────────────────────────────────────────
tmux send-keys -t "=$OUTER:" Enter
check "Enter engages: chip shows keys → e2e-alpha/main" \
  wait_for 10 outer_has 'keys → e2e-alpha/main'

# ── Typed command executes in the inner session ──────────────────────────
tmux send-keys -t "=$OUTER:" -l 'echo e2e mark $((21*2))'
tmux send-keys -t "=$OUTER:" Enter
check "typed command executed inside inner e2e-alpha/main session" \
  wait_for 10 sh -c 'tmux capture-pane -p -t "=garage/e2e-alpha/main:" | sed "s/ *\$//" | grep -qx "e2e mark 42"'

# ── Ctrl+G disengage ─────────────────────────────────────────────────────
tmux send-keys -t "=$OUTER:" C-g
check "Ctrl+G disengages: chip back to keys → garage" \
  wait_for 10 outer_has 'keys → garage'

# ── Alt+Right byte-exactness against keyecho ─────────────────────────────
tmux send-keys -t "=$OUTER:" -l 2
wait_for 15 outer_has 'keyecho ready' || fail "refocus e2e-beta before byte tests"
tmux send-keys -t "=$OUTER:" Enter
wait_for 10 outer_has 'keys → e2e-beta/keyecho' || fail "engage keyecho tile"
tmux send-keys -t "=$OUTER:" -H 1b 5b 31 3b 33 43   # Alt+Right: ESC [ 1 ; 3 C
check "Alt+Right arrives byte-exact in inner pane (^[[1;3C)" \
  wait_for 10 inner_has 'garage/e2e-beta/keyecho' '^[[1;3C'

# ── Shift+Tab passthrough ────────────────────────────────────────────────
tmux send-keys -t "=$OUTER:" -H 1b 5b 5a            # Shift+Tab: ESC [ Z
check "Shift+Tab arrives byte-exact in inner pane (^[[Z)" \
  wait_for 10 inner_has 'garage/e2e-beta/keyecho' '^[[Z'
tmux send-keys -t "=$OUTER:" C-g
wait_for 10 outer_has 'keys → garage' || fail "disengage after byte tests"

# ── n spawns a session ───────────────────────────────────────────────────
tmux send-keys -t "=$OUTER:" -l 1
sleep 0.5
tmux send-keys -t "=$OUTER:" -l n
check "n spawns session (claude-1 appears in rail)" \
  wait_for 15 outer_has 'claude-1'
check "spawned session exists in tmux" \
  wait_for 5 tmux has-session -t '=garage/e2e-alpha/claude-1'

# ── needs-input via hook token ───────────────────────────────────────────
hook() { curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/hooks/claude?token=$TOKEN" \
  -H 'content-type: application/json' -d "$1"; }
NOTIF_BODY='{"hook_event_name":"Notification","message":"e2e question","cwd":"'"$WS_BETA"'"}'
NOTIF_CODE=$(hook "$NOTIF_BODY")
check "hook Notification accepted (200)" test "$NOTIF_CODE" = 200
check "rail amber ordering: blocked e2e-beta bubbles to tab 1" \
  wait_for 10 outer_has '1:e2e-beta'
check "strip shows blocked count" \
  wait_for 10 outer_has 'blocked'

# ── Triage queue: message + Enter jump lands engaged ─────────────────────
clear_foreign_blocked
tmux send-keys -t "=$OUTER:" -l A
check "A opens triage queue (chip keys → queue)" \
  wait_for 10 outer_has 'keys → queue'
check "queue row shows daemon message text" \
  wait_for 10 outer_has 'e2e question'
tmux send-keys -t "=$OUTER:" Enter
check "queue Enter jumps engaged onto blocked session" \
  wait_for 10 outer_has 'keys → e2e-beta/keyecho'
tmux send-keys -t "=$OUTER:" C-g
wait_for 10 outer_has 'keys → garage' || fail "disengage after queue jump"

# ── hook Stop clears ─────────────────────────────────────────────────────
STOP_BODY='{"hook_event_name":"Stop","cwd":"'"$WS_BETA"'"}'
STOP_CODE=$(hook "$STOP_BODY")
check "hook Stop accepted (200)" test "$STOP_CODE" = 200
check "Stop clears needs-input (blocked badge gone, e2e-alpha back to tab 1)" \
  wait_for 10 sh -c "tmux capture-pane -p -t '=$OUTER:' | grep -qF '1:e2e-alpha' && ! tmux capture-pane -p -t '=$OUTER:' | grep -qF 'blocked'"

# ═════════════════════════════════════════════════════════════════════════
# 10.2 Load test: 5 stress sessions, latency at 200x55 and 250x70
# ═════════════════════════════════════════════════════════════════════════
for i in 1 2 3 4 5; do
  spawn e2e-alpha "stress-$i" >/dev/null
  tmux send-keys -t "=garage/e2e-alpha/stress-$i:" -l "exec $STRESS"
  tmux send-keys -t "=garage/e2e-alpha/stress-$i:" Enter
done
check "5 stress sessions spawned and streaming" \
  wait_for 15 sh -c "tmux capture-pane -p -t "=garage/e2e-alpha/stress-1:" | grep -q 'tool call'"
tmux send-keys -t "=$OUTER:" -l 1
check "grid streams under load (stress output visible in tiles)" \
  wait_for 20 outer_has 'tool call'
sleep 3  # let attaches settle before measuring

METRICS=$WORK/metrics.txt
: > "$METRICS"

tui_pid() { pgrep -nf "$TUI_BIN"; }

measure_latency() { # measure_latency <tag> — 5 samples, digit-in-garage-layer
  local tag=$1 samples=() i base t0 t1 lat
  for i in 1 2 3 4 5; do
    base=$(wc -l < "$KEYLOG" 2>/dev/null || echo 0)
    t0=$(now_us)
    tmux send-keys -t "=$OUTER:" -l 1
    if ! wait_for 3 sh -c "[ \$(wc -l < '$KEYLOG' 2>/dev/null || echo 0) -gt $base ]"; then
      echo "latency[$tag] sample $i: keylog line never appeared" | tee -a "$METRICS"
      return 1
    fi
    t1=$(tail -1 "$KEYLOG" | awk '{print $1}')
    lat=$(( (t1 - t0) / 1000 ))
    samples+=("$lat")
    echo "latency[$tag] sample $i: ${lat} ms" | tee -a "$METRICS"
    sleep 0.4
  done
  MEDIAN=$(printf '%s\n' "${samples[@]}" | sort -n | sed -n 3p)
  echo "latency[$tag] median: ${MEDIAN} ms" | tee -a "$METRICS"
}

sample_cpu() { # sample_cpu <tag>
  local tag=$1 pid
  pid=$(tui_pid) || { echo "cpu[$tag]: no TUI pid" | tee -a "$METRICS"; return 1; }
  local vals=()
  for _ in 1 2 3; do
    vals+=("$(ps -o %cpu= -p "$pid" | tr -d ' ')")
    sleep 1
  done
  echo "cpu[$tag] %cpu samples: ${vals[*]}" | tee -a "$METRICS"
}

measure_latency 200x55 || fail "latency sampling at 200x55"
check "keypress-to-handled median < 50 ms at 200x55 (measured ${MEDIAN} ms)" \
  test "$MEDIAN" -lt 50
sample_cpu 200x55

# Quit the 200x55 instance cleanly (also exercises the q-quit contract).
tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit-s200' ] && [ \"\$(cat '$WORK/tui-exit-s200')\" = 0 ]"
check "tmux sessions survive TUI quit" \
  sh -c "tmux has-session -t '=garage/e2e-alpha/main' && tmux has-session -t '=garage/e2e-beta/keyecho'"
tmux kill-session -t "=$OUTER:" 2>/dev/null

# ── 250x70 instance ──────────────────────────────────────────────────────
OUTER=e2e-tui250
launch_tui "$OUTER" 250 70 s250
check "TUI renders at 250x70 under load" \
  wait_for 20 outer_has 'e2e-alpha'
tmux send-keys -t "=$OUTER:" -l 1
wait_for 20 outer_has 'tool call' || fail "stress tiles streaming at 250x70"
sleep 3

measure_latency 250x70 || fail "latency sampling at 250x70"
check "keypress-to-handled median < 50 ms at 250x70 (measured ${MEDIAN} ms)" \
  test "$MEDIAN" -lt 50
sample_cpu 250x70

tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0 at 250x70" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit-s250' ] && [ \"\$(cat '$WORK/tui-exit-s250')\" = 0 ]"

# Deliberately ordered last (historical: the Dart-era binary had a known
# termios-restore gap here; the Rust wall restores the tty fully and these
# pass — kept last so a regression still yields full metrics above).
check "tty restored after quit at 200x55 (no -ixon/-isig/-icanon/-echo left)" \
  sh -c "! grep -qE '(^| )-(ixon|isig|icanon|echo)( |\$)' '$WORK/tui-stty-s200'"
check "tty restored after quit at 250x70 (no -ixon/-isig/-icanon/-echo left)" \
  sh -c "! grep -qE '(^| )-(ixon|isig|icanon|echo)( |\$)' '$WORK/tui-stty-s250'"

echo
echo "ALL $CHECKS_RUN CHECKS PASSED — metrics in $METRICS, results in $RESULTS"
