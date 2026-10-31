#!/bin/sh
# @@TITLE@@: `greet` pushes one text item to the `status` widget.
# Needs only sh and curl. The daemon sets GARAGE_API, GARAGE_PLUGIN_ID and
# GARAGE_PLUGIN_TOKEN; the run context is JSON in $GARAGE_CONTEXT.
set -eu

CURL=${CURL:-curl}

# Escape a string for use inside a JSON string.
json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr -d '\000-\037'
}

push_widget() { # push_widget <widget> <text>
  body=$(printf '{"scopes":{"*":[{"segments":[{"text":"%s"}]}]}}' "$(json_escape "$2")")
  "$CURL" --silent --show-error --fail -X PUT \
    -H "Authorization: Bearer $GARAGE_PLUGIN_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$body" \
    "$GARAGE_API/api/plugins/$GARAGE_PLUGIN_ID/widgets/$1" >/dev/null
}

case "${1:-}" in
  greet) push_widget status "hello from @@TITLE@@" ;;
  *) echo "usage: plugin.sh greet" >&2; exit 2 ;;
esac
