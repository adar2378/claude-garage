#!/bin/bash
# Scrolling colored output, ~40 lines/sec — simulates a chatty agent turn
i=0
while true; do
  c=$((31 + i % 7))
  printf '\033[%sm[%05d]\033[0m tool call \033[1m%s\033[0m — reading \033[36m%s\033[0m +%d −%d\n' \
    "$c" "$i" "Edit" "daemon/src/poller_$((i%9)).js" $((RANDOM%90)) $((RANDOM%40))
  i=$((i+1))
  sleep 0.025
done
