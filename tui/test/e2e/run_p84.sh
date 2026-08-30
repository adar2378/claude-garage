#!/usr/bin/env bash
# p8.4 kill-all removal confirm — e2e harness for the garage TUI.
#
# Covers (spec tui-key-routing "p8.4 kill-all removal confirm"):
#   A. `X`-`X` stays registry-only: the arm notice carries the new
#      `· K to also kill its <n> sessions` clause, the confirm removes the
#      registration, and BOTH tmux sessions stay alive.
#   B. `X` then `K` kills for real: DELETE /api/workspaces/<name>?sessions=kill
#      — the registration is gone AND both tmux sessions are gone; the strip
#      reports `removed <name> · killed 2 sessions`.
#   C. `K` outside an active arm is an ordinary unbound key (nothing
#      removed, nothing killed).
#   D. Zero live sessions: the arm notice omits the `K` clause (p8.3
#      wording unchanged).
#
# Safety contract (fully scratch — the user's live daemon on 4747 is never
# touched): scratch port (GARAGE_E2E_PORT, default 4796) + scratch
# GARAGE_DIR (so ~/.garage is never read or written); refuses to run when
# the port is taken; only tmux sessions named p84-tui / garage/p84-* are
# created or killed; every destructive confirm is gated on the strip
# notice naming the expected p84 workspace first.
#
# Exits non-zero on the first failed check; prints PASS/FAIL per check.

set -u -o pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../../.." && pwd)
TUI_BIN=$REPO/tui/dist/garage-tui-darwin-$(node -p 'process.arch' 2>/dev/null || echo arm64)
WORK=${E2E_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/garage-tui-p84.XXXXXX")}
RESULTS=$WORK/results.txt
mkdir -p "$WORK"
: > "$RESULTS"

PORT=${GARAGE_E2E_PORT:-4796}
BASE=http://127.0.0.1:$PORT
SCRATCH_GARAGE_DIR=$WORK/garage-home
mkdir -p "$SCRATCH_GARAGE_DIR"
DAEMON_PID=""
OUTER=p84-tui

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

ws_registered()   { curl -s -m 2 "$BASE/api/workspaces" | grep -qF "\"$1\""; }
ws_unregistered() { ! ws_registered "$1"; }

# Focus a workspace via its rail digit (tmux is global: the user's real
# garage/* sessions synthesize foreign groups, so the index is dynamic).
focus_ws() { # focus_ws <name>
  local idx
  idx=$(cap | grep -oE "[0-9]+ $1" | head -1 | awk '{print $1}')
  [ -n "$idx" ] || fail "workspace $1 not found in the rail"
  note "focusing workspace $idx $1"
  tmux send-keys -t "=$OUTER:" -l "$idx"
}

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p84-tui|garage/p84-)' \
    | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done
  if [ -n "$DAEMON_PID" ]; then kill "$DAEMON_PID" 2>/dev/null; wait "$DAEMON_PID" 2>/dev/null; fi
  # Fully scratch: state lived in $SCRATCH_GARAGE_DIR; ~/.garage untouched.
  exit "$rc"
}
trap cleanup EXIT INT TERM

# ── Preflight ────────────────────────────────────────────────────────────
[ -x "$TUI_BIN" ] || { echo "TUI binary missing: $TUI_BIN"; exit 2; }
for tool in tmux node curl; do
  command -v "$tool" >/dev/null || { echo "missing tool: $tool"; exit 2; }
done
if curl -s -m 2 "$BASE/api/health" | grep -q ok; then
  echo "refusing to run: something is already listening on $BASE (set GARAGE_E2E_PORT)"; exit 2
fi
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p84-tui|garage/p84-)' \
  | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done

# ── Daemon up (scratch port + scratch GARAGE_DIR, zsh sessions) ──────────
GARAGE_PORT=$PORT GARAGE_DIR=$SCRATCH_GARAGE_DIR GARAGE_CLAUDE_CMD=/bin/zsh \
  node "$REPO/daemon/src/index.js" >"$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
check "scratch daemon up on :$PORT with scratch GARAGE_DIR" \
  wait_for 10 sh -c "curl -s -m 1 $BASE/api/health | grep -q ok"

# ── Workspaces + sessions ────────────────────────────────────────────────
put_ws() { curl -s -o /dev/null -w '%{http_code}' -X PUT "$BASE/api/workspaces" \
  -H 'content-type: application/json' -d "{\"name\":\"$1\",\"dir\":\"$2\"}"; }
spawn() { curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/sessions" \
  -H 'content-type: application/json' -d "{\"workspace\":\"$1\",\"label\":\"$2\"}"; }
for ws in p84-keep p84-kill p84-empty; do
  mkdir -p "$WORK/$ws"
  check "PUT workspace $ws" test "$(put_ws "$ws" "$WORK/$ws")" = 200
done
check "spawn p84-keep/main"   test "$(spawn p84-keep main)"   = 201
check "spawn p84-keep/second" test "$(spawn p84-keep second)" = 201
check "spawn p84-kill/alpha"  test "$(spawn p84-kill alpha)"  = 201
check "spawn p84-kill/beta"   test "$(spawn p84-kill beta)"   = 201

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
check "p84-keep appears in the rail" wait_for 20 sh -c \
  "tmux capture-pane -p -t '=$OUTER:' | grep -qE '[0-9]+ p84-keep'"

# ── 1. X-X on p84-keep: arm notice has the K clause; sessions stay alive ─
focus_ws p84-keep
sleep 0.5
tmux send-keys -t "=$OUTER:" -l X
check "first X arms with the full p8.4 notice (K clause, 2 sessions)" \
  wait_for 5 outer_has 'press X again to remove p84-keep (sessions keep running) · K to also kill its 2 sessions'
tmux send-keys -t "=$OUTER:" -l X
check "X-X removes the registration (gone from /api/workspaces)" \
  wait_for 10 ws_unregistered p84-keep
check "X-X left tmux session main alive"   tmux has-session -t '=garage/p84-keep/main'
check "X-X left tmux session second alive" tmux has-session -t '=garage/p84-keep/second'

# ── 2. K outside an active arm is unbound ────────────────────────────────
focus_ws p84-kill
sleep 0.5
tmux send-keys -t "=$OUTER:" -l K
sleep 1
ws_registered p84-kill || fail "bare K removed a workspace"
pass "bare K removed nothing (p84-kill still registered)"
check "bare K killed nothing (alpha still alive)" \
  tmux has-session -t '=garage/p84-kill/alpha'

# ── 3. X then K kills both sessions and removes the workspace ────────────
tmux send-keys -t "=$OUTER:" -l X
check "X arms on p84-kill with the K clause" \
  wait_for 5 outer_has 'press X again to remove p84-kill (sessions keep running) · K to also kill its 2 sessions'
tmux send-keys -t "=$OUTER:" -l K
check "X-K removes the registration (gone from /api/workspaces)" \
  wait_for 10 ws_unregistered p84-kill
check "kill notice reports the count" \
  wait_for 5 outer_has 'removed p84-kill · killed 2 sessions'
check "tmux session alpha is gone" \
  wait_for 10 sh -c "! tmux has-session -t '=garage/p84-kill/alpha' 2>/dev/null"
check "tmux session beta is gone" \
  wait_for 10 sh -c "! tmux has-session -t '=garage/p84-kill/beta' 2>/dev/null"

# ── 4. Zero live sessions: the arm notice omits the K clause ─────────────
focus_ws p84-empty
sleep 0.5
tmux send-keys -t "=$OUTER:" -l X
check "X arms on the empty workspace with the plain p8.3 notice" \
  wait_for 5 outer_has 'press X again to remove p84-empty (sessions keep running)'
check "the K clause is omitted at zero sessions" \
  outer_lacks 'K to also kill its'
tmux send-keys -t "=$OUTER:" -l z
sleep 0.5
ws_registered p84-empty || fail "disarmed X removed the empty workspace"
pass "z disarmed the empty-workspace arm (still registered)"

# ── 5. Quit ──────────────────────────────────────────────────────────────
tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"

echo
echo "ALL $CHECKS_RUN CHECKS PASSED — results in $RESULTS (work dir $WORK)"
