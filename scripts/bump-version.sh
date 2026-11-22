#!/usr/bin/env bash
# Bumps the app version in the workspace Cargo.toml (the build number is automatic).
# Usage: scripts/bump-version.sh [patch|minor|major]   (default: patch)
set -euo pipefail

source "$(dirname "$0")/env.sh"
PART="${1:-patch}"
CURRENT="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
IFS=. read -r MAJOR MINOR PATCH <<<"$CURRENT"
case "$PART" in
  major) NEXT="$((MAJOR + 1)).0.0" ;;
  minor) NEXT="$MAJOR.$((MINOR + 1)).0" ;;
  patch) NEXT="$MAJOR.$MINOR.$((PATCH + 1))" ;;
  *) echo "usage: $0 [patch|minor|major]" >&2; exit 1 ;;
esac
# Only the first `version = ` line: the one in [workspace.package].
awk -v old="version = \"$CURRENT\"" -v new="version = \"$NEXT\"" \
  '!done && $0 == old { print new; done = 1; next } { print }' Cargo.toml >Cargo.toml.tmp
mv Cargo.toml.tmp Cargo.toml
cargo update --workspace --quiet
echo "$CURRENT -> $NEXT"
