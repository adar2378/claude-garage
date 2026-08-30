#!/bin/bash
for i in 1 2 3 4 5 6 7 8 9; do tmux kill-session -t garage-spike-$i 2>/dev/null; done
echo "spike sessions killed"
