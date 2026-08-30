#!/bin/bash
# In-place spinner with CR overwrites, like Claude Code's working state
frames='⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'
i=0
while true; do
  f=${frames:$((i%10)):1}
  printf '\r\033[33m%s\033[0m Simmering… (%ds · ↓ %d tokens) \033[2mesc to interrupt\033[0m ' "$f" $((i/10)) $((i*37))
  i=$((i+1))
  sleep 0.1
done
