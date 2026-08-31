#!/usr/bin/env bash
# p8.2 post-review fixes — e2e harness.
#
# Covers the two user-reported fixes:
#   A. Launcher stale-daemon gate (bin/garage.js): a pre-upgrade daemon
#      (health without `version`) on the garage port is detected, stopped
#      by pid, and replaced by a current daemon — sessions untouched.
#   B. Engaged inner cursor (vendored nocterm patch 9): an engaged live
#      tile paints the inner terminal's cursor as one inverse-video cell
#      (asserted via `tmux capture-pane -e` SGR parsing from the OUTER
#      session); it follows typing, survives an alt-screen app, and is
#      absent while frozen (Shift+PageUp) and while unengaged.
#
# Safety contract (fully scratch — the user's live daemon on 4747 is never
# touched): scratch port (GARAGE_E2E_PORT, default 4796), scratch
# GARAGE_DIR (so ~/.garage is never read or written — the launcher's
# daemon.log honors GARAGE_DIR too), only tmux sessions named p82-tui /
# garage/p82-* are created or killed, and the only processes signaled are
# the stub/daemon this script itself put on the scratch port.
#
# Exits non-zero on the first failed check; prints PASS/FAIL per check.

set -u -o pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../../.." && pwd)
# Binary under test: GARAGE_TUI_BIN overrides the preflight target and is
# forwarded into the launcher's environment (this harness starts the TUI via
# `bin/garage.js tui`, which resolves the binary itself — p9 lookup order);
# default is the Rust dist binary (wall/dist).
TUI_BIN=${GARAGE_TUI_BIN:-$REPO/wall/dist/garage-wall-darwin-$(node -p 'process.arch' 2>/dev/null || echo arm64)}
WORK=${E2E_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/garage-wall-p82.XXXXXX")}
RESULTS=$WORK/results.txt
mkdir -p "$WORK"
: > "$RESULTS"

PORT=${GARAGE_E2E_PORT:-4796}
BASE=http://127.0.0.1:$PORT
SCRATCH_GARAGE_DIR=$WORK/garage-home
mkdir -p "$SCRATCH_GARAGE_DIR"
OUTER=p82-tui
STUB_PID=""
CAPLOOP_PID=""
VER=$(node -p "require('$REPO/package.json').version")

CHECKS_RUN=0
note() { printf '     %s\n' "$*"; }
pass() { CHECKS_RUN=$((CHECKS_RUN+1)); printf 'PASS %s\n' "$*" | tee -a "$RESULTS"; }
fail() {
  CHECKS_RUN=$((CHECKS_RUN+1)); printf 'FAIL %s\n' "$*" | tee -a "$RESULTS"
  tmux capture-pane -e -p -t "=$OUTER:" > "$WORK/fail-capture.txt" 2>/dev/null
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

cap()       { tmux capture-pane -p -t "=$OUTER:" 2>/dev/null; }
outer_has() { cap | grep -qF -- "$1"; }

# ── SGR reverse-cell parser (the cursor evidence) ────────────────────────
# Prints "row col char" for every screen cell painted with SGR 7 (inverse
# video) in `tmux capture-pane -e` output of the OUTER session — i.e. the
# cells the TUI itself asked the terminal to invert.
cat > "$WORK/revcells.py" <<'PY'
import re, sys
sgr = re.compile(r'\x1b\[([0-9;]*)m')
for row, line in enumerate(sys.stdin.buffer.read().decode('utf-8', 'replace').split('\n')):
    col = 0
    i = 0
    rev = False
    while i < len(line):
        m = sgr.match(line, i)
        if m:
            params = m.group(1)
            codes = [int(p) for p in params.split(';') if p] or [0]
            # Walk codes consuming extended-color sequences whole: per
            # ECMA-48/ANSI, 38;5;N / 48;5;N (indexed) and 38;2;R;G;B /
            # 48;2;R;G;B (truecolor) are ONE color parameter — their payload
            # numbers are palette indices, not SGR attributes. Without this,
            # a foreground of indexed color 7 (e.g. 38;5;7, ratatui's Gray)
            # false-positives as SGR 7 inverse video.
            k = 0
            while k < len(codes):
                c = codes[k]
                if c in (38, 48, 58):
                    if k + 1 < len(codes) and codes[k + 1] == 5:
                        k += 3
                    elif k + 1 < len(codes) and codes[k + 1] == 2:
                        k += 5
                    else:
                        k += 1
                    continue
                if c == 0:
                    rev = False
                elif c == 7:
                    rev = True
                elif c == 27:
                    rev = False
                k += 1
            i = m.end()
            continue
        ch = line[i]
        if ch == '\x1b':          # any other escape: skip it and its final
            i += 2
            continue
        if rev:
            print(row, col, repr(ch))
        col += 1
        i += 1
PY
cursor_cells() { tmux capture-pane -e -p -t "=$OUTER:" 2>/dev/null | python3 "$WORK/revcells.py"; }
cursor_count_is() { [ "$(cursor_cells | wc -l | tr -d ' ')" = "$1" ]; }
cursor_at() { cursor_cells | awk '{print $1, $2}' | grep -qx -- "$1"; }

# The pid serving the scratch port's health endpoint (current daemons
# report it; used for targeted cleanup — never a name-based kill).
health_pid() { curl -s -m 2 "$BASE/api/health" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("pid",""))' 2>/dev/null; }

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  [ -n "$CAPLOOP_PID" ] && kill "$CAPLOOP_PID" 2>/dev/null
  tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p82-tui|garage/p82-)' \
    | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done
  # Stop whatever this script put on the scratch port: the stub, then the
  # launcher-started daemon (by the pid its own health reports).
  [ -n "$STUB_PID" ] && kill "$STUB_PID" 2>/dev/null
  local dpid
  dpid=$(health_pid)
  [ -n "$dpid" ] && kill "$dpid" 2>/dev/null
  # Fully scratch: state + daemon.log lived in $SCRATCH_GARAGE_DIR.
  exit "$rc"
}
trap cleanup EXIT INT TERM

# ── Preflight ────────────────────────────────────────────────────────────
[ -x "$TUI_BIN" ] || { echo "TUI binary missing: $TUI_BIN"; exit 2; }
for tool in tmux node curl python3 lsof vim; do
  command -v "$tool" >/dev/null || { echo "missing tool: $tool"; exit 2; }
done
if curl -s -m 2 "$BASE/api/health" | grep -q ok; then
  echo "refusing to run: something is already listening on $BASE (set GARAGE_E2E_PORT)"; exit 2
fi
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p82-tui|garage/p82-)' \
  | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done

# ── A. Stale-daemon gate ─────────────────────────────────────────────────
# A pre-upgrade daemon: serves /api/health WITHOUT version (and without
# pid), exactly like the daemons that produced the ?meta=1 404s.
cat > "$WORK/stub.js" <<'JS'
const http = require("node:http");
http
  .createServer((req, res) => {
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ status: "ok" }));
  })
  .listen(Number(process.env.PORT), "127.0.0.1");
JS
PORT=$PORT node "$WORK/stub.js" &
STUB_PID=$!
check "old-shaped stub daemon (health without version) up on :$PORT" \
  wait_for 5 sh -c "curl -s -m 1 $BASE/api/health | grep -q '\"status\":\"ok\"'"

# Launch the real launcher (tui path) against the scratch port. The stale
# line prints to the outer pane before the TUI takes the screen — a capture
# loop preserves that startup scroll for the assertion below.
cat > "$WORK/launch.sh" <<EOF
#!/bin/sh
node "$REPO/bin/garage.js" tui
echo \$? > "$WORK/tui-exit"
sleep 120
EOF
chmod +x "$WORK/launch.sh"
tmux new-session -d -x 200 -y 55 \
  -e GARAGE_PORT="$PORT" -e GARAGE_TUI_PORT="$PORT" \
  -e GARAGE_TUI_BIN="${GARAGE_TUI_BIN:-}" \
  -e GARAGE_DIR="$SCRATCH_GARAGE_DIR" -e GARAGE_CLAUDE_CMD='/bin/zsh -f' \
  -s "$OUTER" "$WORK/launch.sh"
( while :; do tmux capture-pane -p -t "=$OUTER:" 2>/dev/null; sleep 0.1; done ) \
  > "$WORK/startup-scroll.txt" 2>/dev/null &
CAPLOOP_PID=$!

check "launcher swaps the stub out (stub pid $STUB_PID gone)" \
  wait_for 20 sh -c "! kill -0 $STUB_PID 2>/dev/null"
check "replacement daemon reports the launcher's version ($VER)" \
  wait_for 20 sh -c "curl -s -m 1 $BASE/api/health | grep -qF '\"version\":\"$VER\"'"
check "replacement daemon reports its pid" \
  sh -c "curl -s -m 1 $BASE/api/health | grep -qF '\"pid\":'"
check "TUI comes up against the replacement daemon (strip chip renders)" \
  wait_for 20 outer_has 'keys → garage'
kill "$CAPLOOP_PID" 2>/dev/null; wait "$CAPLOOP_PID" 2>/dev/null; CAPLOOP_PID=""
check "stale line printed (names both versions)" \
  grep -q 'is stale (launcher v' "$WORK/startup-scroll.txt"
check "stale line says sessions are untouched" \
  grep -q 'sessions are untouched' "$WORK/startup-scroll.txt"
note "daemon.log stayed in the scratch GARAGE_DIR ($(ls "$SCRATCH_GARAGE_DIR" | tr '\n' ' '))"
[ -f "$SCRATCH_GARAGE_DIR/daemon.log" ] || fail "launcher wrote daemon.log into the scratch GARAGE_DIR"
pass "launcher wrote daemon.log into the scratch GARAGE_DIR"

# ── B. Engaged inner cursor ──────────────────────────────────────────────
# One session running `/bin/zsh -f` (clean prompt, no rc files — the
# reverse-video SGR count on screen must be cursor-only).
curl -s -o /dev/null -X PUT "$BASE/api/workspaces" \
  -H 'content-type: application/json' -d "{\"name\":\"p82-a\",\"dir\":\"$WORK\"}"
SPAWN_CODE=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/sessions" \
  -H 'content-type: application/json' -d '{"workspace":"p82-a","label":"main"}')
check "spawn p82-a/main" test "$SPAWN_CODE" = 201

# tmux is global: the user's real garage/* sessions synthesize foreign
# workspace groups on this wall (same caveat as run_p81.sh), and focus may
# sit on one of them — focus p82-a via its rail number before asserting on
# grid content.
check "p82-a appears in the rail" wait_for 20 sh -c \
  "tmux capture-pane -p -t '=$OUTER:' | grep -qE '[0-9]+ p82-a'"
WS_IDX=$(cap | grep -oE '[0-9]+ p82-a' | head -1 | awk '{print $1}')
note "focusing workspace $WS_IDX p82-a"
tmux send-keys -t "=$OUTER:" -l "$WS_IDX"
check "tile attaches (zsh prompt renders in the tile)" \
  wait_for 20 outer_has '%'

check "unengaged tile shows NO cursor (zero inverse-video cells)" \
  wait_for 10 cursor_count_is 0

# Engage: the sole tile is focused; Enter dispatches EngageCommand.
tmux send-keys -t "=$OUTER:" Enter
check "Enter engages (chip keys → p82-a/main)" \
  wait_for 10 outer_has 'keys → p82-a/main'
check "engaged live tile shows EXACTLY ONE inverse-video cell (the cursor)" \
  wait_for 10 cursor_count_is 1
POS0=$(cursor_cells | awk '{print $1, $2}')
ROW0=${POS0% *}; COL0=${POS0#* }
note "cursor cell at row $ROW0 col $COL0 (capture-pane -e SGR 7)"

# Typing forwards to the PTY; the cursor must follow (6 chars → +6 cols).
tmux send-keys -t "=$OUTER:" -l 'abcdef'
check "cursor follows typing (row $ROW0, col $COL0 → $((COL0+6)))" \
  wait_for 10 cursor_at "$ROW0 $((COL0+6))"
check "still exactly one cursor cell after typing" cursor_count_is 1

# Frozen scrollback (Shift+PageUp) must hide the cursor; End snaps live.
# Two subtleties: a fresh tile has no history above the viewport and
# Shift+PageUp then stays live BY DESIGN (ScrollAnchor.scrollUp); and
# instant bulk output (a bare `seq`) finishes inside one tmux redraw
# cycle, so tmux just repaints the final screen and no scroll ever reaches
# the emulator's buffer — output must be SLOW enough that tmux actually
# scrolls it through the attach client.
tmux send-keys -t "=$OUTER:" C-u
tmux send-keys -t "=$OUTER:" -l 'for i in $(seq 1 120); do echo $i; sleep 0.02; done'
tmux send-keys -t "=$OUTER:" Enter
wait_for 20 outer_has '119' || fail "slow scrollback output landed in the tile"
sleep 1
tmux send-keys -t "=$OUTER:" S-PPage
check "frozen view opens (title shows the back-to-live affordance)" \
  wait_for 10 outer_has '↓ live'
check "frozen scrollback shows NO cursor" wait_for 5 cursor_count_is 0
tmux send-keys -t "=$OUTER:" End
check "End snaps back to live — cursor returns" wait_for 10 cursor_count_is 1

# Alt-screen app: vim repositions the cursor (top-left of the file view,
# vs. end-of-prompt for the shell) and paints no reverse video of its own
# (unlike less, whose standout prompt would pollute the count) — the single
# inverted cell must track the app. C-u first clears the typed abcdef.
tmux send-keys -t "=$OUTER:" C-u
tmux send-keys -t "=$OUTER:" -l 'vim -u NONE /etc/hosts'
tmux send-keys -t "=$OUTER:" Enter
alt_cursor_moved() {
  cursor_count_is 1 || return 1
  [ "$(cursor_cells | awk '{print $1, $2}')" != "$POS0" ]
}
check "alt-screen app (vim): still exactly one cursor cell, repositioned" \
  wait_for 15 alt_cursor_moved
tmux send-keys -t "=$OUTER:" -l ':q!'
tmux send-keys -t "=$OUTER:" Enter
check "back at the shell after vim (one cursor cell at the new prompt)" \
  wait_for 15 cursor_count_is 1

# Disengage: no cursor on the unengaged wall.
tmux send-keys -t "=$OUTER:" C-g
check "Ctrl+G disengages (chip keys → garage)" \
  wait_for 10 outer_has 'keys → garage'
check "unengaged again: NO cursor cell" wait_for 5 cursor_count_is 0

# ── Quit ─────────────────────────────────────────────────────────────────
tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"

# ── C. Stale-daemon gate, pid path ──────────────────────────────────────
# Stage A exercised the lsof fallback (pre-upgrade daemons report no pid).
# Current-but-wrong-version daemons DO report a pid — the launcher must use
# it directly. Swap the scratch daemon for a versioned stub reporting its
# own pid and run the launcher again.
DPID=$(health_pid)
[ -n "$DPID" ] && kill "$DPID" 2>/dev/null
wait_for 10 sh -c "! curl -s -m 1 $BASE/api/health | grep -q ok" \
  || fail "scratch daemon released the port for stage C"
cat > "$WORK/stub2.js" <<'JS'
const http = require("node:http");
http
  .createServer((req, res) => {
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ status: "ok", version: "0.0.1", pid: process.pid }));
  })
  .listen(Number(process.env.PORT), "127.0.0.1");
JS
PORT=$PORT node "$WORK/stub2.js" &
STUB_PID=$!
check "versioned stub daemon (v0.0.1, reports pid $STUB_PID) up on :$PORT" \
  wait_for 5 sh -c "curl -s -m 1 $BASE/api/health | grep -qF '\"version\":\"0.0.1\"'"

tmux kill-session -t "=$OUTER" 2>/dev/null
rm -f "$WORK/tui-exit"
tmux new-session -d -x 200 -y 55 \
  -e GARAGE_PORT="$PORT" -e GARAGE_TUI_PORT="$PORT" \
  -e GARAGE_TUI_BIN="${GARAGE_TUI_BIN:-}" \
  -e GARAGE_DIR="$SCRATCH_GARAGE_DIR" -e GARAGE_CLAUDE_CMD='/bin/zsh -f' \
  -s "$OUTER" "$WORK/launch.sh"
( while :; do tmux capture-pane -p -t "=$OUTER:" 2>/dev/null; sleep 0.1; done ) \
  > "$WORK/startup-scroll2.txt" 2>/dev/null &
CAPLOOP_PID=$!
check "launcher kills the versioned stub via its reported pid" \
  wait_for 20 sh -c "! kill -0 $STUB_PID 2>/dev/null"
check "replacement daemon is current again ($VER)" \
  wait_for 20 sh -c "curl -s -m 1 $BASE/api/health | grep -qF '\"version\":\"$VER\"'"
check "TUI comes back up" wait_for 20 outer_has 'keys → garage'
kill "$CAPLOOP_PID" 2>/dev/null; wait "$CAPLOOP_PID" 2>/dev/null; CAPLOOP_PID=""
check "stale line names the old version (v0.0.1)" \
  grep -q 'daemon v0.0.1 is stale (launcher v' "$WORK/startup-scroll2.txt"
tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0 (stage C)" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"

echo
echo "ALL $CHECKS_RUN CHECKS PASSED — results in $RESULTS (work dir $WORK)"
