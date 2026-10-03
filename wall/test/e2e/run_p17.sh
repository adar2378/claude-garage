#!/usr/bin/env bash
# p17 worktree finish overlay + `I` hooks install — e2e harness for the TUI.
#
# Covers (specs tui-worktree-finish, tui-hooks-install, worktree-sessions):
#   A. plain session x-x: no overlay
#   B. worktree x-x -> overlay names branch -> target; m merges and cleans up
#   C. d d discards (first d only arms)
#   D. dirty worktree: m shows the error inline, overlay stays; k keeps all
#   E. merge conflict: error inline, repo not left mid-merge; Esc keeps
#   F. I installs hooks + statusline into a scratch GARAGE_CLAUDE_HOME;
#      second I reports "hooks already installed"
#
# Safety: scratch port 4797, scratch GARAGE_DIR and GARAGE_CLAUDE_HOME
# (~/.garage and ~/.claude untouched), zsh instead of claude in sessions,
# scratch git repo; only p17-tui / garage/p17-* tmux sessions are touched.

set -u -o pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../../.." && pwd)
# Binary under test: GARAGE_TUI_BIN overrides (p9 parity gate points it at
# the Rust binary — same checks); default is the Rust dist binary (wall/dist).
TUI_BIN=${GARAGE_TUI_BIN:-$REPO/wall/dist/garage-wall-darwin-$(node -p 'process.arch' 2>/dev/null || echo arm64)}
WORK=${E2E_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/garage-wall-p17.XXXXXX")}
RESULTS=$WORK/results.txt
mkdir -p "$WORK"
: > "$RESULTS"

PORT=${GARAGE_E2E_PORT:-4797}
BASE=http://127.0.0.1:$PORT
SCRATCH_GARAGE_DIR=$WORK/garage-home
mkdir -p "$SCRATCH_GARAGE_DIR" "$WORK/claude-home"
echo "{}" > "$WORK/claude-home/settings.json"
DAEMON_PID=""
OUTER=p17-tui

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
  tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p17-tui|garage/p17-)' \
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
tmux ls -F '#{session_name}' 2>/dev/null | grep -E '^(p17-tui|garage/p17-)' \
  | while IFS= read -r s; do tmux kill-session -t "=$s" 2>/dev/null; done

# ── Daemon up (scratch port + scratch GARAGE_DIR, zsh sessions) ──────────
GARAGE_PORT=$PORT GARAGE_DIR=$SCRATCH_GARAGE_DIR GARAGE_CLAUDE_HOME=$WORK/claude-home GARAGE_CLAUDE_CMD=/bin/zsh \
  node "$REPO/daemon/src/index.js" >"$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
check "scratch daemon up on :$PORT with scratch GARAGE_DIR" \
  wait_for 10 sh -c "curl -s -m 1 $BASE/api/health | grep -q ok"


REPO_DIR=$WORK/repo
git init -q -b main "$REPO_DIR"
git -C "$REPO_DIR" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
echo base > "$REPO_DIR/file.txt"
git -C "$REPO_DIR" add file.txt
git -C "$REPO_DIR" -c user.email=t@t -c user.name=t commit -q -m base
GIT="git -c user.email=t@t -c user.name=t"

curl -s -o /dev/null -X PUT "$BASE/api/workspaces" -H 'content-type: application/json' \
  -d "{\"name\":\"p17-wt\",\"dir\":\"$REPO_DIR\"}"
spawn() { # spawn <label> <worktree true|false>
  curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/sessions" \
    -H 'content-type: application/json' \
    -d "{\"workspace\":\"p17-wt\",\"label\":\"$1\",\"worktree\":$2}"; }
wt_path() { echo "$SCRATCH_GARAGE_DIR/worktrees/p17-wt/$1"; }
has_branch() { git -C "$REPO_DIR" show-ref --verify --quiet "refs/heads/$1"; }
no_merge_in_progress() { [ ! -f "$REPO_DIR/.git/MERGE_HEAD" ]; }
key() { tmux send-keys -t "=$OUTER:" -l "$1"; }

cat > "$WORK/launch.sh" <<EOF
#!/bin/sh
"$TUI_BIN"
echo \$? > "$WORK/tui-exit"
sleep 120
EOF
chmod +x "$WORK/launch.sh"
tmux new-session -d -x 200 -y 55 -e GARAGE_TUI_PORT="$PORT" -s "$OUTER" "$WORK/launch.sh"
check "TUI launches against the scratch daemon" wait_for 15 outer_has 'keys → garage'
check "p17-wt appears in the rail" wait_for 20 sh -c \
  "tmux capture-pane -p -t '=$OUTER:' | grep -qE '[0-9]+ p17-wt'"
focus_ws p17-wt
sleep 0.5

close_focused() { # close_focused <label>
  key x
  wait_for 5 outer_has "press x again to close $1" || fail "x did not arm close for $1"
  key x
}

# ── A. plain session: no overlay ─────────────────────────────────────────
check "spawn plain session" test "$(spawn plain false)" = 201
check "plain tile renders" wait_for 10 outer_has 'plain'
close_focused plain
check "plain session gone" wait_for 10 sh -c "! tmux has-session -t '=garage/p17-wt/plain' 2>/dev/null"
sleep 1
check "no finish overlay for a plain session" outer_lacks 'finish worktree'

# ── B. merge ─────────────────────────────────────────────────────────────
check "spawn worktree session wtm" test "$(spawn wtm true)" = 201
W=$(wt_path wtm)
check "worktree dir created" test -d "$W"
echo merged > "$W/new.txt"; git -C "$W" add new.txt; $GIT -C "$W" commit -q -m "wtm work"
check "wtm tile renders" wait_for 10 outer_has 'wtm'
close_focused wtm
check "overlay opens naming branch and target" wait_for 10 outer_has 'garage/wtm → main'
check "overlay offers merge into main" outer_has 'merge into main'
key m
check "merge notice" wait_for 15 outer_has 'merged garage/wtm into main'
check "overlay closed after merge" outer_lacks 'finish worktree'
check "merge commit landed on main" test -f "$REPO_DIR/new.txt"
check "branch garage/wtm deleted" sh -c "! git -C '$REPO_DIR' show-ref --verify --quiet refs/heads/garage/wtm"
check "worktree dir removed" test ! -d "$W"

# ── C. discard ───────────────────────────────────────────────────────────
check "spawn worktree session wtd" test "$(spawn wtd true)" = 201
W=$(wt_path wtd)
echo x > "$W/d.txt"; git -C "$W" add d.txt; $GIT -C "$W" commit -q -m "wtd work"
check "wtd tile renders" wait_for 10 outer_has 'wtd'
close_focused wtd
check "overlay opens for wtd" wait_for 10 outer_has 'garage/wtd → main'
key d
check "first d only arms" wait_for 5 outer_has 'press d again to discard garage/wtd'
has_branch garage/wtd || fail "first d deleted the branch"
pass "branch still present after one d"
key d
check "discard notice" wait_for 15 outer_has 'discarded garage/wtd'
check "branch garage/wtd force-deleted" sh -c "! git -C '$REPO_DIR' show-ref --verify --quiet refs/heads/garage/wtd"
check "wtd worktree dir removed" test ! -d "$W"

# ── D. dirty worktree: inline error, k keeps ─────────────────────────────
check "spawn worktree session wtx" test "$(spawn wtx true)" = 201
W=$(wt_path wtx)
echo dirty > "$W/dirty.txt"
check "wtx tile renders" wait_for 10 outer_has 'wtx'
close_focused wtx
check "overlay opens for wtx" wait_for 10 outer_has 'garage/wtx → main'
key m
check "dirty error shown inline" wait_for 10 outer_has 'uncommitted changes'
check "overlay still open after error" outer_has 'finish worktree'
key k
check "k closes the overlay" wait_for 5 outer_lacks 'finish worktree'
check "k kept branch garage/wtx" has_branch garage/wtx
check "k kept the worktree dir" test -d "$W"

# ── E. conflict: inline error, repo not mid-merge, Esc keeps ─────────────
check "spawn worktree session wtc" test "$(spawn wtc true)" = 201
W=$(wt_path wtc)
echo theirs > "$W/file.txt"; $GIT -C "$W" commit -qam "wtc change"
echo ours > "$REPO_DIR/file.txt"; $GIT -C "$REPO_DIR" commit -qam "main change"
check "wtc tile renders" wait_for 10 outer_has 'wtc'
close_focused wtc
check "overlay opens for wtc" wait_for 10 outer_has 'garage/wtc → main'
key m
check "conflict error shown inline" wait_for 15 outer_has 'merge conflict'
check "repo not left mid-merge" no_merge_in_progress
check "repo working tree clean" test -z "$(git -C "$REPO_DIR" status --porcelain)"
cap > "$WORK/conflict-capture.txt"
tmux send-keys -t "=$OUTER:" Escape
check "Esc closes the overlay" wait_for 5 outer_lacks 'finish worktree'
check "Esc kept branch garage/wtc" has_branch garage/wtc

# ── F. I installs hooks + statusline (scratch claude home) ───────────────
key I
check "I notice: hooks installed" wait_for 15 outer_has 'hooks installed ·'
check "scratch settings.json has the garage hook" grep -q "/api/hooks/claude" "$WORK/claude-home/settings.json"
cap > "$WORK/install-capture.txt"
sleep 9
key I
check "second I: hooks already installed" wait_for 15 outer_has 'hooks already installed'
check "real ~/.claude/settings.json untouched by the run" sh -c "! grep -q ':$PORT/' \"\$HOME/.claude/settings.json\""

key q
check "q quits TUI with exit code 0" \
  wait_for 10 sh -c "[ -f '$WORK/tui-exit' ] && [ \"\$(cat '$WORK/tui-exit')\" = 0 ]"
# Scratch worktrees left by k/Esc live under $WORK; drop them from the repo.
echo
echo "ALL $CHECKS_RUN CHECKS PASSED — results in $RESULTS (work dir $WORK)"
