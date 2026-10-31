#!/bin/sh
# Runs plugin.sh against a fake curl that records what would be sent.
set -eu
dir=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/curl" <<'FAKE'
#!/bin/sh
printf '%s\n' "$@" > "$CURL_ARGS"
FAKE
chmod +x "$tmp/curl"

CURL="$tmp/curl" CURL_ARGS="$tmp/args" \
GARAGE_API=http://127.0.0.1:1 GARAGE_PLUGIN_ID=@@ID@@ GARAGE_PLUGIN_TOKEN=tok \
  sh "$dir/plugin.sh" greet

grep -q -- '^http://127.0.0.1:1/api/plugins/@@ID@@/widgets/status$' "$tmp/args" || { echo "FAIL: wrong URL"; cat "$tmp/args"; exit 1; }
grep -q -- '^Authorization: Bearer tok$' "$tmp/args" || { echo "FAIL: no token"; exit 1; }
grep -q -- '"text":"hello from @@TITLE@@"' "$tmp/args" || { echo "FAIL: wrong body"; exit 1; }
echo "ok"
