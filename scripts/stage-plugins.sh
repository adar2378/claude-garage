#!/bin/sh
# Stage the bundled first-party plugins (crates in plugins/<name>/) as
#   <dest>/<name>/garage-plugin.toml
#   <dest>/<name>/bin/<name>
# which is the layout the daemon discovers (plugins/ next to the claude-garage
# binary; see crates/daemon/src/plugins/bundled.rs).
#
#   scripts/stage-plugins.sh release wall/dist/plugins   # npm package
#   scripts/stage-plugins.sh release target/release/plugins
#   scripts/stage-plugins.sh debug                       # dev: bins into plugins/<name>/bin (gitignored)
#
# Build first (`cargo build [--release] --workspace`); this only copies.
set -eu
profile="${1:?usage: stage-plugins.sh <debug|release> [dest]}"
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="${2:-}"
bins="${CARGO_TARGET_DIR:-$root/target}/$profile"

for dir in "$root"/plugins/*/; do
  [ -f "$dir/garage-plugin.toml" ] || continue
  name="$(basename "$dir")"
  out="${dest:+$dest/$name}"
  out="${out:-$dir}"
  mkdir -p "$out/bin"
  if [ -f "$bins/$name" ]; then
    cp "$bins/$name" "$out/bin/$name"
  fi
  if [ -n "$dest" ]; then
    cp "$dir/garage-plugin.toml" "$out/garage-plugin.toml"
  fi
  echo "staged $name -> $out"
done
