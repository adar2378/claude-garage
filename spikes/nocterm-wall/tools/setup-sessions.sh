#!/bin/bash
# Creates the 9 tmux sessions the spike wall attaches to.
cd "$(dirname "$0")/.." || exit 1
D=$PWD
for i in 1 2 3 4 5; do
  tmux kill-session -t garage-spike-$i 2>/dev/null
  tmux new-session -d -s garage-spike-$i -c "$D/sandbox/ws-alpha" "$D/tools/stress.sh"
done
for i in 6 7; do
  tmux kill-session -t garage-spike-$i 2>/dev/null
  tmux new-session -d -s garage-spike-$i -c "$D/sandbox/ws-beta" "$D/tools/spinner.sh"
done
tmux kill-session -t garage-spike-8 2>/dev/null
tmux new-session -d -s garage-spike-8 -c "$D/sandbox/ws-beta" "$D/tools/keyecho.sh"
tmux kill-session -t garage-spike-9 2>/dev/null
tmux new-session -d -s garage-spike-9 -c "$D/sandbox/ws-beta"
echo "9 sessions ready:"; tmux ls | grep garage-spike
echo "now run: bin/wall"
