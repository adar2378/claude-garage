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
# Build first (`cargo build [--release] --workspace`); this only copies, and
# fails when a plugin's binary is missing. A plugin without a Cargo.toml is
# manifest-only and ships just the manifest. `hello` is a dev/test fixture and
# is never staged.
set -eu
profile="${1:?usage: stage-plugins.sh <debug|release> [dest]}"
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="${2:-}"
bins="${CARGO_TARGET_DIR:-$root/target}/$profile"

for dir in "$root"/plugins/*/; do
  [ -f "$dir/garage-plugin.toml" ] || continue
  name="$(basename "$dir")"
  [ "$name" = hello ] && continue
  if [ ! -f "$dir/Cargo.toml" ]; then
    # manifest-only plugin (no commands): ships just its manifest
    if [ -n "$dest" ]; then
      mkdir -p "$dest/$name"
      cp "$dir/garage-plugin.toml" "$dest/$name/garage-plugin.toml"
      echo "staged $name (manifest only) -> $dest/$name"
    fi
    continue
  fi
  out="${dest:+$dest/$name}"
  out="${out:-$dir}"
  if [ ! -f "$bins/$name" ]; then
    echo "stage-plugins: missing binary $bins/$name (run cargo build first)" >&2
    exit 1
  fi
  mkdir -p "$out/bin"
  cp "$bins/$name" "$out/bin/$name"
  if [ -n "$dest" ]; then
    cp "$dir/garage-plugin.toml" "$out/garage-plugin.toml"
  fi
  echo "staged $name -> $out"
done
