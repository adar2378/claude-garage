#!/usr/bin/env bash
# Framework-level click smoke: drives click_probe.dart in a scratch tmux
# session with injected SGR mouse sequences. Touches NO daemon and NO
# ~/.garage state — companion to run_click_smoke.sh (the full-app smoke,
# which needs port 4747 free).
#
# Usage: run_click_probe_smoke.sh [path-to-compiled-probe]
#        (default: compiles test/e2e/click_probe.dart into $TMPDIR)

set -u -o pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
TUI_DIR=$(cd "$SCRIPT_DIR/../.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/garage-click-probe.XXXXXX")
PROBE=${1:-$WORK/click_probe}
OUTER=e2e-click-probe

CHECKS_RUN=0
pass() { CHECKS_RUN=$((CHECKS_RUN+1)); printf 'PASS %s\n' "$*"; }
fail() {
  CHECKS_RUN=$((CHECKS_RUN+1)); printf 'FAIL %s\n' "$*"
  tmux capture-pane -p -t "=$OUTER:" 2>/dev/null | sed -n 1,32p
  exit 1
}
check() { local desc=$1; shift; if "$@"; then pass "$desc"; else fail "$desc"; fi; }

wait_for() {
  local timeout=$1 t0; shift
  t0=$(date +%s)
  while true; do
    if "$@"; then return 0; fi
    [ $(( $(date +%s) - t0 )) -ge "$timeout" ] && return 1
    sleep 0.2
  done
}

cap()       { tmux capture-pane -p -t "=$OUTER:" 2>/dev/null; }
outer_has() { cap | grep -qF -- "$1"; }

click() { # click <x> <y> — 1-based SGR press+release
  local hex
  hex=$(python3 -c 'import sys
x, y = sys.argv[1], sys.argv[2]
s = f"\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m"
print(" ".join(f"{b:02x}" for b in s.encode()))' "$1" "$2")
  # shellcheck disable=SC2086
  tmux send-keys -t "=$OUTER:" -H $hex
}

line_of() { # 1-based capture line where <needle> first appears
  cap | grep -nF -- "$1" | head -1 | cut -d: -f1
}
col_of() { # 1-based char column of <needle> in line <lineno>
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
  tmux kill-session -t "=$OUTER" 2>/dev/null
  exit "$rc"
}
trap cleanup EXIT INT TERM

[ -x "$PROBE" ] || (cd "$TUI_DIR" && dart compile exe test/e2e/click_probe.dart -o "$PROBE" >/dev/null) \
  || { echo "probe compile failed"; exit 2; }
tmux kill-session -t "=$OUTER" 2>/dev/null

# 100x30: rail 28 wide, grid 72 wide at cols 29..100, strip on line 30.
tmux new-session -d -x 100 -y 30 -s "$OUTER" "$PROBE"
check "probe renders" wait_for 15 outer_has 'LAST=[none]'

# Grid: SGR (60,10) → local col 60-1-28=31, row 9; region reports 72x29.
click 60 10
check "grid click yields local coords (grid:31,9/72x29)" \
  wait_for 5 outer_has 'LAST=[grid:31,9/72x29]'

# Rail: content row 1 ('  row one') renders on capture line 2 (border
# inset) — click there, expect local row 1.
ROW=$(line_of '  row one'); [ -n "$ROW" ] || fail "rail row not found"
click 5 "$ROW"
check "rail click yields local row (rail:1)" wait_for 5 outer_has 'LAST=[rail:1]'

# Badge on the strip (line 30).
BCOL=$(col_of 'blocked' 30); [ -n "$BCOL" ] || fail "badge col not found"
click "$BCOL" 30
check "badge click resolves (badge)" wait_for 5 outer_has 'LAST=[badge]'

# Overlay: modal absorbs, barrier yields, wall underneath is blocked.
tmux send-keys -t "=$OUTER:" -l o
wait_for 5 outer_has 'queue row A' || fail "overlay did not open"
QROW=$(line_of 'queue row A'); [ -n "$QROW" ] || fail "modal row not found"
click 50 "$QROW"
check "modal row click absorbed with border+padding offset (modal:2)" \
  wait_for 5 outer_has 'LAST=[modal:2]'
click 4 4
check "outside click reaches the barrier (barrier)" \
  wait_for 5 outer_has 'LAST=[barrier]'
click 60 10   # over the grid, but the overlay is up — must NOT hit grid
sleep 1
check "overlay blocks the wall underneath (still barrier, no grid hit)" \
  outer_has 'LAST=[barrier]'
tmux send-keys -t "=$OUTER:" -l o
wait_for 5 sh -c "! tmux capture-pane -p -t '=$OUTER:' | grep -qF 'queue row A'" \
  || fail "overlay did not close"

# Repeated clicks keep firing (press/release state machine resets).
click 60 10
check "click works again after overlay round-trip" \
  wait_for 5 outer_has 'LAST=[grid:31,9/72x29]'

tmux send-keys -t "=$OUTER:" -l q
echo
echo "ALL $CHECKS_RUN CHECKS PASSED"
