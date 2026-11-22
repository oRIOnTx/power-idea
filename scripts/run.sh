#!/usr/bin/env bash
# Runs the IDE (debug build) on a project.
# Usage: scripts/run.sh [project-dir]   (defaults to this repository)
set -euo pipefail

PROJECT="$(cd "${1:-.}" && pwd)"
source "$(dirname "$0")/env.sh"
cargo run -p power-idea -- "$PROJECT"
