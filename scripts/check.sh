#!/usr/bin/env bash
# The quick loop: formats, lints (warnings are errors) and tests the workspace; must pass
# before a change is done. `just check` runs everything CI runs.
# Usage: scripts/check.sh [--live]   (--live also runs the ruby-lsp tests)
set -euo pipefail

source "$(dirname "$0")/env.sh"
cargo fmt
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
if [[ "${1:-}" == "--live" ]]; then
  cargo test -p lsp -- --ignored
fi
