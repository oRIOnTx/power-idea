#!/usr/bin/env bash
# Formats, lints and tests the workspace; must pass before a change is done.
# Usage: scripts/check.sh [--live]   (--live also runs the ruby-lsp tests)
set -euo pipefail

source "$(dirname "$0")/env.sh"
cargo fmt
cargo clippy --all-targets
cargo test
if [[ "${1:-}" == "--live" ]]; then
  cargo test -p lsp -- --ignored
fi
