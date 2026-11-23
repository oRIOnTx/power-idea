#!/usr/bin/env bash
# The app version, computed from git — the single source of truth (never edit it by hand).
#
#   VERSION  marketing version from the latest annotated tag v* (v1.5.0 → 1.5.0)
#   BUILD    git rev-list --count HEAD (monotonic, the same on every machine)
#   SHA      short commit hash
#   FULL     release (HEAD on the tag, clean): "1.5.0 (412)"
#            dev: "1.5.0-dev.3+abc1234" (+ ".dirty" with uncommitted changes)
#
# Usage: scripts/version.sh [--json]     prints shell-sourceable KEY=value lines, or JSON.
# APP_VERSION / APP_BUILD / GIT_SHA override git (Docker, CI without history). Never fails.
# VERSION_REPO points it at another repository (tests).
set -uo pipefail
cd "${VERSION_REPO:-$(dirname "${BASH_SOURCE[0]}")/..}" 2>/dev/null || true

FIRST_VERSION="0.1.0"

from_git() {
  git rev-parse --verify --quiet HEAD >/dev/null 2>&1 || return 1
  SHA="$(git rev-parse --short=7 HEAD)"
  BUILD="$(git rev-list --count HEAD)"
  local describe ahead
  # Annotated tags only (no --tags), so lightweight tags never become versions.
  if describe="$(git describe --long --match 'v[0-9]*' 2>/dev/null)"; then
    # v1.5.0-3-gabc1234 → tag v1.5.0, 3 commits after it
    local tag="${describe%-*-g*}"
    ahead="${describe#"$tag"-}"
    ahead="${ahead%%-*}"
    VERSION="${tag#v}"
  else
    echo "version.sh: no v* tag yet; using $FIRST_VERSION — create the first one with bin/release" >&2
    VERSION="$FIRST_VERSION"
    ahead="$BUILD"
  fi
  local dirty=""
  git diff --quiet HEAD -- 2>/dev/null || dirty=".dirty"
  if [[ "$ahead" == "0" && -z "$dirty" ]]; then
    FULL="$VERSION ($BUILD)"
    RELEASE=true
  else
    FULL="$VERSION-dev.$ahead+$SHA$dirty"
    RELEASE=false
  fi
}

if [[ -n "${APP_VERSION:-}" ]] || ! from_git; then
  VERSION="${APP_VERSION:-$FIRST_VERSION}"
  BUILD="${APP_BUILD:-0}"
  SHA="${GIT_SHA:-unknown}"
  FULL="$VERSION ($BUILD)"
  RELEASE=true
fi

if [[ "${1:-}" == "--json" ]]; then
  printf '{"version":"%s","build":%s,"sha":"%s","full":"%s","release":%s}\n' \
    "$VERSION" "$BUILD" "$SHA" "$FULL" "$RELEASE"
else
  printf 'VERSION=%q\nBUILD=%q\nSHA=%q\nFULL=%q\nRELEASE=%q\n' "$VERSION" "$BUILD" "$SHA" "$FULL" "$RELEASE"
fi
