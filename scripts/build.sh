#!/usr/bin/env bash
# Builds the release "Power IDEA.app" and opens it.
# Usage: scripts/build.sh [--no-open]
set -euo pipefail

"$(dirname "$0")/bundle-macos.sh"
if [[ "${1:-}" != "--no-open" ]]; then
  open "$(dirname "$0")/../target/release/Power IDEA.app"
fi
