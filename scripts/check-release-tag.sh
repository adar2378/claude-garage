#!/bin/sh
# Fails unless the release tag names the workspace version.
#
#   scripts/check-release-tag.sh <tag>      e.g. v1.0.0-rc.1, 1.0.0, releases/v1.0.0
#
# Run by the release gate (.github/workflows/release-tests.yml) before anything
# is built or published. The workspace version (Cargo.toml) and package.json
# ("keep in step") must also agree with each other.
set -eu

tag="${1:?usage: check-release-tag.sh <tag>}"
cd "$(dirname "$0")/.."

# vX.Y.Z[-pre] or X.Y.Z[-pre], optionally under a prefix like releases/
want="${tag##*/}"
want="${want#v}"

have=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\([^"]*\)".*/\1/p;}' Cargo.toml | head -n 1)
[ -n "$have" ] || { echo "error: no [workspace.package] version in Cargo.toml" >&2; exit 2; }

npm=$(sed -n 's/^  "version": "\([^"]*\)".*/\1/p' package.json | head -n 1)

fail=0
if [ "$want" != "$have" ]; then
  echo "error: tag $tag says version $want but Cargo.toml has $have" >&2
  fail=1
fi
if [ "$npm" != "$have" ]; then
  echo "error: package.json has version ${npm:-<none>} but Cargo.toml has $have" >&2
  fail=1
fi
[ "$fail" -eq 0 ] && echo "ok: $tag is workspace version $have"
exit "$fail"
