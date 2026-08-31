#!/bin/bash
# Echo raw received bytes visibly — passthrough verification target
echo "keyecho ready — bytes appear below:"
stty raw -echo 2>/dev/null
cat -v
