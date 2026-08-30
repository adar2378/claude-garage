#!/usr/bin/env bash
# ratatui-wall spike — gate harness (mirrors tui/test/e2e/run_e2e.sh style).
#
# Pattern: the compiled wall runs inside a detached scratch tmux session
# (rspike-outer*); keys are injected with `tmux send-keys` (raw hex for escape
# sequences) and assertions read `tmux capture-pane -p` of the OUTER pane
# (wall chrome / tile regions) or the INNER rspike sessions (byte-exactness).
#
# Safety contract:
#   - Only tmux sessions named rspike-* are created or killed.
#   - No daemon, no ~/.garage, no network. The user's sessions are untouched.
#
# Exits non-zero on the first failed check; prints PASS/FAIL per check.

set -u -o pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
SPIKE=$(cd "$SCRIPT_DIR/.." && pwd)
BIN=$SPIKE/target/release/ratatui-wall
STRESS=$SPIKE/tools/stress.sh
SPINNER=$SPIKE/tools/spinner.sh
KEYECHO=$SPIKE/tools/keyecho.sh
WORK=${SPIKE_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/ratatui-wall.XXXXXX")}
KEYLOG=$WORK/keylog.txt
RESULTS=$WORK/results.txt
METRICS=$WORK/metrics.txt
mkdir -p "$WORK"; : > "$RESULTS"; : > "$METRICS"

CHECKS_RUN=0
note()  { printf '     %s\n' "$*"; }
pass()  { CHECKS_RUN=$((CHECKS_RUN+1)); printf 'PASS %s\n' "$*" | tee -a "$RESULTS"; }
fail()  {
  CHECKS_RUN=$((CHECKS_RUN+1)); printf 'FAIL %s\n' "$*" | tee -a "$RESULTS"
  tmux capture-pane -p -t "=${OUTER:-rspike-outer}:" > "$WORK/fail-capture.txt" 2>/dev/null
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

outer_has()  { tmux capture-pane -p -t "=$OUTER:" 2>/dev/null | grep -qF -- "$1"; }
outer_lacks(){ ! outer_has "$1"; }
inner_has()  { tmux capture-pane -p -J -t "=$1:" 2>/dev/null | grep -qF -- "$2"; }
now_us() { perl -MTime::HiRes=time -e 'printf "%d", time()*1000000'; }

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^rspike-' \
    | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done
  pkill -f "$BIN" 2>/dev/null
  exit "$rc"
}
trap cleanup EXIT INT TERM

# ── Preflight ────────────────────────────────────────────────────────────
[ -x "$BIN" ] || { echo "wall binary missing: $BIN (cargo build --release)"; exit 2; }
for tool in tmux perl awk; do
  command -v "$tool" >/dev/null || { echo "missing tool: $tool"; exit 2; }
done
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^rspike-' \
  | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done

# ── Scratch sessions: 5 stress + 2 spinner + keyecho + shell ─────────────
for i in 1 2 3 4 5; do tmux new-session -d -s "rspike-$i" -x 100 -y 30 "$STRESS"; done
for i in 6 7;       do tmux new-session -d -s "rspike-$i" -x 100 -y 30 "$SPINNER"; done
tmux new-session -d -s rspike-8 -x 100 -y 30 "$KEYECHO"
tmux new-session -d -s rspike-9 -x 100 -y 30
check "9 rspike sessions up" \
  test "$(tmux ls -F '#{session_name}' | grep -cE '^rspike-[1-9]$')" = 9

launch_wall() { # launch_wall <session-name> <cols> <rows> <tag>
  local name=$1 cols=$2 rows=$3 tag=$4
  cat > "$WORK/launch-$tag.sh" <<EOF
#!/bin/sh
"$BIN"
echo \$? > "$WORK/exit-$tag"
stty -a < /dev/tty > "$WORK/stty-$tag" 2>&1
sleep 180
EOF
  chmod +x "$WORK/launch-$tag.sh"
  tmux new-session -d -x "$cols" -y "$rows" \
    -e GARAGE_TUI_KEYLOG="$KEYLOG" -s "$name" "$WORK/launch-$tag.sh"
}

OUTER=rspike-outer
launch_wall "$OUTER" 200 55 s200
check "wall renders 3x3 grid (rspike-1 title visible, 200x55)" \
  wait_for 15 outer_has '1:rspike-1'
check "stress output streams in tiles" \
  wait_for 20 outer_has 'tool call'
check "spinner tile streams" \
  wait_for 10 outer_has 'Simmering'
check "chip shows keys → garage" \
  wait_for 5 outer_has 'keys → garage'
sleep 3  # let attaches + resizes settle

# ── Gate 1a: resize — tile PTYs sized to tiles (TIOCSWINSZ) ──────────────
# 200 cols → 3 tiles ≈ 66/67 wide → inner 64/65; grid 54 rows → 18 → inner 16.
CW=$(tmux list-clients -t =rspike-1 -F '#{client_width}x#{client_height}' | head -1)
note "rspike-1 attach client size: $CW (tile inner expected ~66x16)"
check "tile PTY resized from 80x24 default (width 60-70, height 14-18)" \
  sh -c "echo '$CW' | grep -qE '^(6[0-9]|70)x(1[4-8])\$'"
CW5=$(tmux list-clients -t =rspike-5 -F '#{client_width}x#{client_height}' | head -1)
note "rspike-5 attach client size: $CW5"
check "middle-row tile PTY also sized" \
  sh -c "echo '$CW5' | grep -qE '^(6[0-9]|70)x(1[4-8])\$'"

# ── Gate 1b: latency + CPU under load at 200x55 ──────────────────────────
wall_pid() { pgrep -nf "$BIN"; }

measure_latency() { # 5 samples, digit-in-garage-layer, epoch-us keylog diff
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

sample_cpu() {
  local tag=$1 pid
  pid=$(wall_pid) || { echo "cpu[$tag]: no wall pid" | tee -a "$METRICS"; return 1; }
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

# ── Gate 2: verbatim passthrough against keyecho (tile 8) ────────────────
tmux send-keys -t "=$OUTER:" -l 8
sleep 0.3
tmux send-keys -t "=$OUTER:" Enter
check "Enter engages tile 8 (chip keys → rspike-8)" \
  wait_for 5 outer_has 'keys → rspike-8'

tmux send-keys -t "=$OUTER:" -l h
check "plain h arrives byte-exact" wait_for 5 inner_has rspike-8 'h'
tmux send-keys -t "=$OUTER:" C-a
check "Ctrl+A arrives as ^A (0x01)" wait_for 5 inner_has rspike-8 '^A'
tmux send-keys -t "=$OUTER:" C-c
check "Ctrl+C arrives as ^C (0x03), wall survives" wait_for 5 inner_has rspike-8 '^C'
tmux send-keys -t "=$OUTER:" -H 1b 5b 31 3b 33 43   # Alt+Right: ESC [1;3C
check "Alt+Right arrives byte-exact (^[[1;3C — modifier survives)" \
  wait_for 5 inner_has rspike-8 '^[[1;3C'
tmux send-keys -t "=$OUTER:" -H 1b 66              # Alt+f: ESC f
check "Alt+f arrives byte-exact (^[f)" wait_for 5 inner_has rspike-8 '^[f'
tmux send-keys -t "=$OUTER:" -H 1b 5b 5a           # Shift+Tab: ESC [Z
check "Shift+Tab arrives byte-exact (^[[Z)" wait_for 5 inner_has rspike-8 '^[[Z'
tmux send-keys -t "=$OUTER:" -H 1b 5b 44           # Left: ESC [D
check "Left arrow arrives byte-exact (^[[D)" wait_for 5 inner_has rspike-8 '^[[D'

# Coalesced multi-key: ONE send-keys -H carrying h, Shift+Tab, Enter, i —
# must arrive as individual correct events (the nocterm three-patch class).
tmux send-keys -t "=$OUTER:" -H 68 1b 5b 5a 0d 69
check "coalesced h+ShiftTab+Enter+i arrive as individual keys (h^[[Z^Mi)" \
  wait_for 5 inner_has rspike-8 'h^[[Z^Mi'

tmux send-keys -t "=$OUTER:" C-g
check "Ctrl+G disengages (chip back to keys → garage)" \
  wait_for 5 outer_has 'keys → garage'
check "Ctrl+G was consumed, not forwarded (no ^G in keyecho)" \
  sh -c "! tmux capture-pane -p -J -t '=rspike-8:' | grep -qF '^G'"

# ── Gate 3: frozen scrollback with absolute anchor (tile 1, streaming) ───
# Tile 1 interior at 200x55: rows 2-17, cols 2-60 (safely inside borders).
tile1() { tmux capture-pane -p -t "=$OUTER:" | sed -n '2,17p' | cut -c2-60; }
tile2() { tmux capture-pane -p -t "=$OUTER:" | sed -n '2,17p' | cut -c72-120; }

tmux send-keys -t "=$OUTER:" -l 1
sleep 0.3
tmux send-keys -t "=$OUTER:" Enter
check "engage streaming tile 1 (chip keys → rspike-1)" \
  wait_for 5 outer_has 'keys → rspike-1'
tmux send-keys -t "=$OUTER:" -H 1b 5b 35 3b 32 7e   # Shift+PageUp: ESC [5;2~
sleep 0.3
tmux send-keys -t "=$OUTER:" -H 1b 5b 35 3b 32 7e   # x2
check "tile title shows frozen state" wait_for 5 outer_has 'engaged·frozen'
sleep 0.5
F1=$(tile1); G1=$(tile2)
sleep 2
F2=$(tile1); G2=$(tile2)
check "frozen tile content byte-identical across 2s (absolute anchor)" \
  test "$F1" = "$F2"
check "another tile advanced meanwhile" test "$G1" != "$G2"
check "tmux copy-mode never triggered (#{pane_in_mode} = 0)" \
  test "$(tmux display-message -p -t '=rspike-1:' -F '#{pane_in_mode}')" = 0
tmux send-keys -t "=$OUTER:" -H 1b 5b 36 3b 32 7e   # Shift+PageDown
sleep 0.3
tmux send-keys -t "=$OUTER:" -H 1b 5b 36 3b 32 7e   # x2 → live
check "Shift+PageDown x2 returns to live (frozen marker gone)" \
  wait_for 5 outer_lacks 'engaged·frozen'
L1=$(tile1); sleep 1.5; L2=$(tile1)
check "live tile streams again after PageDown" test "$L1" != "$L2"
# Freeze once more, then typing snaps to live.
tmux send-keys -t "=$OUTER:" -H 1b 5b 35 3b 32 7e
check "re-frozen for typing-snap test" wait_for 5 outer_has 'engaged·frozen'
tmux send-keys -t "=$OUTER:" -l x
check "typing snaps to live (frozen marker gone)" \
  wait_for 5 outer_lacks 'engaged·frozen'
tmux send-keys -t "=$OUTER:" C-g
wait_for 5 outer_has 'keys → garage' || fail "disengage after scrollback gate"

# ── Quit restores the terminal; sessions survive ─────────────────────────
tmux send-keys -t "=$OUTER:" -l q
check "q quits wall with exit code 0" \
  wait_for 10 sh -c "[ -f '$WORK/exit-s200' ] && [ \"\$(cat '$WORK/exit-s200')\" = 0 ]"
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^rspike-[1-9]$' | sort > "$WORK/sessions-after-quit.txt"
check "tty restored after quit (no -ixon/-isig/-icanon/-echo left)" \
  sh -c "! grep -qE '(^| )-(ixon|isig|icanon|echo)( |,|\$)' '$WORK/stty-s200'"
check "all 9 rspike sessions survive quit" \
  test "$(tmux ls -F '#{session_name}' | grep -cE '^rspike-[1-9]$')" = 9
tmux kill-session -t "=$OUTER:" 2>/dev/null

# ── SIGKILL the wall: sessions must still survive ────────────────────────
OUTER=rspike-outerkill
launch_wall "$OUTER" 200 55 skill
wait_for 15 outer_has '1:rspike-1' || fail "wall up for SIGKILL test"
KPID=$(wall_pid) || fail "wall pid for SIGKILL test"
kill -9 "$KPID"
sleep 1
check "SIGKILLed wall never kills tmux sessions (9 still up)" \
  test "$(tmux ls -F '#{session_name}' | grep -cE '^rspike-[1-9]$')" = 9
tmux kill-session -t "=$OUTER:" 2>/dev/null
pkill -f "$BIN" 2>/dev/null

# ── 250x70 instance: latency + CPU + resize under load ───────────────────
OUTER=rspike-outer250
launch_wall "$OUTER" 250 70 s250
check "wall renders at 250x70 under load" wait_for 20 outer_has '1:rspike-1'
wait_for 20 outer_has 'tool call' || fail "stress tiles streaming at 250x70"
sleep 3
CW250=$(tmux list-clients -t =rspike-1 -F '#{client_width}x#{client_height}' | head -1)
note "rspike-1 attach client size at 250x70: $CW250 (tile inner expected ~81x21)"
check "tile PTY resized for 250x70 (width 78-84, height 19-23)" \
  sh -c "echo '$CW250' | grep -qE '^(7[89]|8[0-4])x(19|2[0-3])\$'"

measure_latency 250x70 || fail "latency sampling at 250x70"
check "keypress-to-handled median < 50 ms at 250x70 (measured ${MEDIAN} ms)" \
  test "$MEDIAN" -lt 50
sample_cpu 250x70

tmux send-keys -t "=$OUTER:" -l q
check "q quits with exit code 0 at 250x70" \
  wait_for 10 sh -c "[ -f '$WORK/exit-s250' ] && [ \"\$(cat '$WORK/exit-s250')\" = 0 ]"
check "tty restored after 250x70 quit" \
  sh -c "! grep -qE '(^| )-(ixon|isig|icanon|echo)( |,|\$)' '$WORK/stty-s250'"

echo
echo "ALL $CHECKS_RUN CHECKS PASSED — metrics in $METRICS, work dir $WORK"
