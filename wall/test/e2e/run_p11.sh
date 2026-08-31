#!/usr/bin/env bash
# p11-context-meters — e2e harness for the garage TUI (task 3.1).
#
# Covers (specs context-telemetry, tui-context-meters):
#   1. Statusline ingest -> tile meter: a fake statusline POST (scratch
#      hookToken, cwd-fallback session resolution — no session_id needed)
#      carrying context_window.used_percentage 42 and 88 renders as the tile
#      bar's ▰/▱ segment meter, dim below 80% and the palette's red at/above
#      80% — asserted via `tmux capture-pane -e` SGR foreground-color codes,
#      not just the text (a session with no post at all shows no meter).
#   2. The same POST's rate_limits render as the strip's `5h N% · wk M%`
#      usage chip.
#   3. Transcript fallback: with GARAGE_CLAUDE_HOME pointed at a scratch
#      `~/.claude`-shaped dir, a crafted transcript JSONL fixture under
#      projects/<cwd-slug>/<claudeSessionId>.jsonl makes the meter appear
#      with NO statusline post ever made for that session — claudeSessionId
#      is injected straight into the scratch GARAGE_DIR's state.json (there
#      is no API for it; the poller only ever learns one from a real `claude
#      agents --json` match, which a zsh-backed e2e session never produces).
#   4. Chaining wrapper byte parity: install over a fake pre-existing
#      statusLine command, then run the generated wrapper script directly
#      with sample stdin and assert its stdout is byte-identical to running
#      the original command alone.
#   5. `I` fires POST /api/statusline/install from the wall and shows the
#      exact success notice text.
#
# A macOS-only environment note (see verification.md "Transcript fixture
# path flake"): tmux's `pane_current_path` was observed to intermittently
# report the raw vs. fully-resolved (symlink-free) form of a scratch dir
# under $TMPDIR across successive queries from the SAME long-lived daemon
# process — not a garage bug, a tmux/macOS quirk specific to a symlinked
# $TMPDIR. The transcript fixture is therefore written under BOTH the raw
# and `realpath`-resolved slug (write_transcript_fixture below) so the test
# never depends on which form a given request happens to see.
#
# Safety contract (fully scratch — the user's live daemon on 4747 and
# ~/.garage/~/.claude are never touched):
#   - runs its own daemon on a SCRATCH PORT (GARAGE_E2E_PORT, default 4800)
#     with a SCRATCH GARAGE_DIR *and* a SCRATCH GARAGE_CLAUDE_HOME, both
#     exported to the daemon AND the wall (wall never reads either today,
#     but both ride along for parity/safety with every other e2e harness
#     here that touches daemon-side scratch state);
#   - refuses to run when something already listens on the chosen port;
#   - only tmux sessions named p11e2e-tui / garage/p11e2e-* are created or
#     killed.
#
# Exits non-zero on the first failed check; prints PASS/FAIL per check.

set -u -o pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../../.." && pwd)
# Binary under test: default is the Rust dist binary (wall/dist); override
# with GARAGE_TUI_BIN.
TUI_BIN=${GARAGE_TUI_BIN:-$REPO/wall/dist/garage-wall-darwin-$(node -p 'process.arch' 2>/dev/null || echo arm64)}
WORK=${E2E_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/garage-wall-p11.XXXXXX")}
RESULTS=$WORK/results.txt
mkdir -p "$WORK"
: > "$RESULTS"

PORT=${GARAGE_E2E_PORT:-4800}
BASE=http://127.0.0.1:$PORT
SCRATCH_GARAGE_DIR=$WORK/garage-home
SCRATCH_CLAUDE_HOME=$WORK/claude-home
mkdir -p "$SCRATCH_GARAGE_DIR" "$SCRATCH_CLAUDE_HOME"
DAEMON_PID=""
OUTER=p11e2e-tui

CHECKS_RUN=0
note() { printf '     %s\n' "$*"; }
pass() { CHECKS_RUN=$((CHECKS_RUN+1)); printf 'PASS %s\n' "$*" | tee -a "$RESULTS"; }
fail() {
  CHECKS_RUN=$((CHECKS_RUN+1)); printf 'FAIL %s\n' "$*" | tee -a "$RESULTS"
  tmux capture-pane -p -e -t "=$OUTER:" > "$WORK/fail-capture.txt" 2>/dev/null
  curl -s -m 2 "$BASE/api/sessions" > "$WORK/fail-sessions.json" 2>/dev/null
  curl -s -m 2 "$BASE/api/usage" > "$WORK/fail-usage.json" 2>/dev/null
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
cap_e()       { tmux capture-pane -p -e -t "=$OUTER:" 2>/dev/null; }
outer_has()   { cap | grep -qF -- "$1"; }
outer_lacks() { ! outer_has "$1"; }

# True when `cap_e`'s raw SGR stream contains the literal escape+text
# sequence "<ESC>[<code>m <meter_text>" — i.e. the exact styled span
# tile.rs's title_line() emits for the context meter (a leading space, then
# `context_meter_text`, immediately preceded by its color's SGR code; see
# wall/src/ui/tile.rs). `code` is a 256-color index ("7" = colors::DIM,
# "1" = colors::CTX_HOT — verified empirically against this binary; see
# verification.md).
meter_color_ok() { # meter_color_ok <sgr_code> <meter_text>
  cap_e | python3 -c "
import sys
data = sys.stdin.read()
needle = '\x1b[' + sys.argv[1] + 'm ' + sys.argv[2]
sys.exit(0 if needle in data else 1)
" "$1" "$2"
}

# The tile title-bar row for a session with this LABEL (labels here are
# unique per session, unlike run_p10's shared "main" — grepping the label
# text unambiguously finds that one tile's own title-bar row, since
# tile.rs's title_line() prints the label verbatim and nothing else on
# screen mentions it).
tile_line_for() { cap | grep -F -- "$1" | head -1; }

# True once a session's tile is actually GRIDDED (rendered as a bordered
# tile with its title bar), not merely listed in the rail — the rail lists
# every session in every workspace regardless of focus (as "   ○ <label>",
# no border), so a bare `outer_has '<label>'` is already true before the
# workspace switch even lands and would let a `wait_for` gate pass on stale
# screen content (exactly the footgun run_p10.sh's own comments call out).
# The tile's OWN top-left border corner immediately followed by its glyph
# and label ("╭ ○ <label>") only ever appears on the actual gridded tile.
gridded() { outer_has "╭ ○ $1"; }

cleanup() {
  local rc=$?
  trap - EXIT
  set +e
  tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p11e2e-tui|garage/p11e2e-)' \
    | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done
  if [ -n "$DAEMON_PID" ]; then kill "$DAEMON_PID" 2>/dev/null; wait "$DAEMON_PID" 2>/dev/null; fi
  # Fully scratch: state lived in $SCRATCH_GARAGE_DIR / $SCRATCH_CLAUDE_HOME;
  # ~/.garage and ~/.claude untouched.
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
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p11e2e-tui|garage/p11e2e-)' \
  | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done

# ── Daemon up (scratch port + scratch GARAGE_DIR + scratch GARAGE_CLAUDE_HOME,
# zsh sessions — statusline.js/transcript.js/settings-install.js all read the
# claudeHome()/GARAGE_DIR overrides fresh per call, but the SETTINGS_PATH
# const in statusline.js is captured ONCE at module load, so the env var
# must be set on this process from the start) ────────────────────────────
GARAGE_PORT=$PORT GARAGE_DIR=$SCRATCH_GARAGE_DIR GARAGE_CLAUDE_HOME=$SCRATCH_CLAUDE_HOME \
  GARAGE_CLAUDE_CMD=/bin/zsh node "$REPO/daemon/src/index.js" >"$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
check "scratch daemon up on :$PORT with scratch GARAGE_DIR + GARAGE_CLAUDE_HOME" \
  wait_for 10 sh -c "curl -s -m 1 $BASE/api/health | grep -q ok"
TOKEN=$(curl -s "$BASE/api/hooks/snippet" | python3 -c 'import json,sys;print(json.load(sys.stdin)["hooks"]["Notification"][0]["hooks"][0]["url"].split("token=")[1])')
statusline_post() { curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/statusline/claude?token=$TOKEN" \
  -H 'content-type: application/json' -d "$1"; }

# ── Workspaces + sessions ─────────────────────────────────────────────────
put_ws() { curl -s -o /dev/null -w '%{http_code}' -X PUT "$BASE/api/workspaces" \
  -H 'content-type: application/json' -d "{\"name\":\"$1\",\"dir\":\"$2\"}"; }
spawn_body() { curl -s -X POST "$BASE/api/sessions" \
  -H 'content-type: application/json' -d "{\"workspace\":\"$1\",\"label\":\"$2\"}"; }
dir_of() { echo "$1" | python3 -c 'import json,sys;print(json.load(sys.stdin)["dir"])'; }

WS_DIM=$WORK/ws-dim     # context 42% — dim meter
WS_HOT=$WORK/ws-hot     # context 88% — red ("hot") meter
WS_NONE=$WORK/ws-none   # no statusline post at all — no meter
WS_TX=$WORK/ws-tx       # transcript-fallback only — no statusline post either
mkdir -p "$WS_DIM" "$WS_HOT" "$WS_NONE" "$WS_TX"

check "PUT workspace p11e2e-dim"  test "$(put_ws p11e2e-dim "$WS_DIM")"   = 200
check "PUT workspace p11e2e-hot"  test "$(put_ws p11e2e-hot "$WS_HOT")"   = 200
check "PUT workspace p11e2e-none" test "$(put_ws p11e2e-none "$WS_NONE")" = 200
check "PUT workspace p11e2e-tx"   test "$(put_ws p11e2e-tx "$WS_TX")"     = 200

DIM_BODY=$(spawn_body p11e2e-dim dim)
check "spawn p11e2e-dim/dim" test -n "$(echo "$DIM_BODY" | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])' 2>/dev/null)"
DIM_DIR=$(dir_of "$DIM_BODY")

HOT_BODY=$(spawn_body p11e2e-hot hot)
check "spawn p11e2e-hot/hot" test -n "$(echo "$HOT_BODY" | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])' 2>/dev/null)"
HOT_DIR=$(dir_of "$HOT_BODY")

NONE_BODY=$(spawn_body p11e2e-none nada)
check "spawn p11e2e-none/nada" test -n "$(echo "$NONE_BODY" | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])' 2>/dev/null)"

TX_BODY=$(spawn_body p11e2e-tx txn)
TX_SID=$(echo "$TX_BODY" | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')
check "spawn p11e2e-tx/txn" test -n "$TX_SID"

# ── 1+2. Statusline POST BEFORE the TUI ever launches — the wall's initial
# GET /api/sessions and GET /api/usage fetches (both fire once at startup,
# no wait needed) already reflect this data, sidestepping GET /api/usage's
# 60s poll cadence entirely (see verification.md "Usage-chip poll cadence").
# rate_limits only take effect on a POST that resolves to a session
# (unresolvable posts are silently ignored, context AND rate limits both —
# see statusline.js) so they ride the dim session's post.
# Built via printf (not nested \"-escaping) — the JSON's own unescaped `{`
# `}` `,` inside a bash double-quoted string are otherwise indistinguishable
# from a brace-expansion group to bash's word-splitting, which silently
# shreds a hand-escaped literal into multiple words (see verification.md
# "JSON payload construction"); same discipline as run_p10.sh's hook().
DIM_PAYLOAD=$(printf '{"cwd":"%s","context_window":{"used_percentage":42},"rate_limits":{"five_hour":{"used_percentage":91,"resets_at":"2026-01-01T00:00:00Z"},"seven_day":{"used_percentage":33,"resets_at":"2026-01-08T00:00:00Z"}}}' "$DIM_DIR")
HOT_PAYLOAD=$(printf '{"cwd":"%s","context_window":{"used_percentage":88}}' "$HOT_DIR")
check "statusline POST: dim context 42% + rate limits (cwd fallback)" \
  test "$(statusline_post "$DIM_PAYLOAD")" = 200
check "statusline POST: hot context 88%" \
  test "$(statusline_post "$HOT_PAYLOAD")" = 200

# ── Launch the TUI (200x55) against the scratch daemon ───────────────────
cat > "$WORK/launch.sh" <<EOF
#!/bin/sh
"$TUI_BIN"
echo \$? > "$WORK/tui-exit"
sleep 120
EOF
chmod +x "$WORK/launch.sh"
launch_tui() {
  tmux new-session -d -x 200 -y 55 \
    -e GARAGE_TUI_PORT="$PORT" -e GARAGE_DIR="$SCRATCH_GARAGE_DIR" \
    -e GARAGE_CLAUDE_HOME="$SCRATCH_CLAUDE_HOME" \
    -s "$OUTER" "$WORK/launch.sh"
}
launch_tui
check "TUI launches against the scratch daemon (strip chip renders)" \
  wait_for 15 outer_has 'keys → garage'

focus_ws() { # focus_ws <name>
  local idx
  idx=$(cap | grep -oE "[0-9]+ $1" | head -1 | awk '{print $1}')
  [ -n "$idx" ] || fail "workspace $1 not found in the rail"
  tmux send-keys -t "=$OUTER:" -l "$idx"
}

# ── 1a. Dim meter (42% -> ceil(42/25)=2 segments, colors::DIM = 38;5;7) ──
focus_ws p11e2e-dim
check "focused p11e2e-dim (dim's own tile is gridded)" \
  wait_for 15 gridded dim
check "dim tile shows the meter text (▰▰▱▱ 42%)" outer_has '▰▰▱▱ 42%'
check "...rendered dim (38;5;7), never red" \
  meter_color_ok '38;5;7' '▰▰▱▱ 42%'

# ── 1b. Hot meter (88% -> 4/4 segments, colors::CTX_HOT = 38;5;1 red) ────
focus_ws p11e2e-hot
check "focused p11e2e-hot (hot's own tile is gridded)" \
  wait_for 15 gridded hot
check "hot tile shows the meter text (▰▰▰▰ 88%)" outer_has '▰▰▰▰ 88%'
check "...rendered red (38;5;1), never dim, never amber" \
  meter_color_ok '38;5;1' '▰▰▰▰ 88%'

# ── 1c. No-data session: no statusline post, no transcript -> no meter ───
focus_ws p11e2e-none
check "focused p11e2e-none (nada's own tile is gridded)" \
  wait_for 15 gridded nada
no_meter_for_nada() { local line; line=$(tile_line_for nada); [ -n "$line" ] && ! printf '%s' "$line" | grep -qF '%'; }
check "no-data session's tile title carries no % at all" no_meter_for_nada

# ── 2. Strip usage chip (account-wide, visible regardless of focus) ──────
check "strip usage chip shows both windows (5h 91% · wk 33%)" \
  outer_has '5h 91% · wk 33%'

# ── 5. `I` installs the statusline wrapper from the wall ─────────────────
tmux send-keys -t "=$OUTER:" -l I
check "I fires the install effect: exact success notice" \
  wait_for 10 outer_has 'statusline feed installed — meters go live as agents work'

tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"
tmux kill-session -t "=$OUTER" 2>/dev/null

# ── 3. Transcript fallback: no statusline post, ever, for this session ───
# claudeSessionId has no API — inject it straight into the scratch
# GARAGE_DIR's state.json (documented in the file header above and in
# verification.md "Injecting claudeSessionId for the fallback test").
TX_CSID=fake-claude-session-id-p11e2e
python3 - "$SCRATCH_GARAGE_DIR/state.json" "$TX_SID" "$TX_CSID" <<'PY'
import json, sys
path, sid, csid = sys.argv[1], sys.argv[2], sys.argv[3]
with open(path) as f:
    state = json.load(f)
state.setdefault("sessions", {})
state["sessions"].setdefault(sid, {})
state["sessions"][sid]["claudeSessionId"] = csid
with open(path, "w") as f:
    json.dump(state, f)
PY
check "claudeSessionId injected into scratch state.json" \
  grep -qF "$TX_CSID" "$SCRATCH_GARAGE_DIR/state.json"

TX_FIXTURE=$WORK/tx-fixture.jsonl
cat > "$TX_FIXTURE" <<'JSONL'
{"type":"user","message":{"role":"user","content":"hi"}}
{"type":"assistant","message":{"model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":80000,"cache_read_input_tokens":15000,"cache_creation_input_tokens":5000}}}
JSONL
# 100000 tokens / the 200000 default window = 50% -> ceil(50/25)=2 segments.

# Writes the fixture under BOTH the raw dir's slug and (if different) its
# realpath-resolved slug — see the file header's environment note.
write_transcript_fixture() { # write_transcript_fixture <dir> <claude_session_id> <content_file>
  local dir=$1 csid=$2 content_file=$3
  local slug=${dir//\//-}
  mkdir -p "$SCRATCH_CLAUDE_HOME/projects/$slug"
  cp "$content_file" "$SCRATCH_CLAUDE_HOME/projects/$slug/$csid.jsonl"
  local resolved
  resolved=$(cd "$dir" 2>/dev/null && pwd -P)
  if [ -n "$resolved" ] && [ "$resolved" != "$dir" ]; then
    local slug2=${resolved//\//-}
    mkdir -p "$SCRATCH_CLAUDE_HOME/projects/$slug2"
    cp "$content_file" "$SCRATCH_CLAUDE_HOME/projects/$slug2/$csid.jsonl"
  fi
}
TX_DIR=$(dir_of "$TX_BODY")
write_transcript_fixture "$TX_DIR" "$TX_CSID" "$TX_FIXTURE"
check "transcript fixture written under $SCRATCH_CLAUDE_HOME/projects" \
  sh -c "find '$SCRATCH_CLAUDE_HOME/projects' -name '$TX_CSID.jsonl' | grep -q ."

# Prime the daemon's 15s cache (each GET kicks a non-blocking background
# refresh — see transcript.js) BEFORE launching the TUI, same "prime before
# launch" discipline as the statusline scenario above.
tx_context_is_50() {
  curl -s "$BASE/api/sessions" | python3 -c "
import json, sys
d = json.load(sys.stdin)
c = next((s.get('context') for s in d if s['id'] == '$TX_SID'), None)
sys.exit(0 if c and c.get('usedPercentage') == 50 and c.get('source') == 'transcript' else 1)
"
}
check "transcript fallback populates context (usedPercentage 50, source transcript) — NO statusline post ever made" \
  wait_for 15 tx_context_is_50

launch_tui
check "TUI relaunches against the scratch daemon" \
  wait_for 15 outer_has 'keys → garage'
focus_ws p11e2e-tx
check "focused p11e2e-tx (txn's own tile is gridded)" \
  wait_for 15 gridded txn
check "transcript-fallback meter renders (▰▰▱▱ 50%) with zero statusline setup" \
  wait_for 10 outer_has '▰▰▱▱ 50%'
check "...rendered dim (38;5;7), same ladder as the statusline-sourced meter" \
  meter_color_ok '38;5;7' '▰▰▱▱ 50%'

tmux send-keys -t "=$OUTER:" -l q
check "q quits TUI with exit code 0 (second run)" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"

# ── 4. Chaining wrapper: byte parity against a fake pre-existing statusline
# No TUI needed — pure daemon + shell. "cat" as the pre-existing command
# makes byte parity trivial to prove: piping ANY sample stdin through the
# generated wrapper must produce that exact stdin back, verbatim (the
# wrapper's own background curl POST is silenced/non-blocking and must
# never leak into stdout — see statusline.js's wrapperScript).
SETTINGS_PATH=$SCRATCH_CLAUDE_HOME/settings.json
cat > "$SETTINGS_PATH" <<'JSON'
{"statusLine": {"type": "command", "command": "cat"}}
JSON
INSTALL_BODY=$(curl -s -X POST "$BASE/api/statusline/install")
check "POST /api/statusline/install captures+chains the fake pre-existing command" \
  test "$(echo "$INSTALL_BODY" | python3 -c 'import json,sys;print(json.load(sys.stdin)["chained"])')" = True
WRAPPER_PATH=$SCRATCH_CLAUDE_HOME/garage-statusline.sh
check "wrapper script written and executable" test -x "$WRAPPER_PATH"

SAMPLE_STDIN='{"context_window":{"used_percentage":77},"nested":{"a":1,"b":[1,2,3]}}'
EXPECTED=$(printf '%s' "$SAMPLE_STDIN" | sh -c 'cat')
ACTUAL=$(printf '%s' "$SAMPLE_STDIN" | sh "$WRAPPER_PATH")
check "wrapper chain byte parity: wrapped stdout == original alone's stdout" \
  test "$ACTUAL" = "$EXPECTED"
check "...and both equal the sample stdin verbatim" \
  test "$ACTUAL" = "$SAMPLE_STDIN"

echo
echo "ALL $CHECKS_RUN CHECKS PASSED — results in $RESULTS (work dir $WORK)"
