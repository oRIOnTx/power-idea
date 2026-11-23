# Developer tasks. `just check` runs everything CI runs (.github/workflows/ci.yml).
# Tools: bin/setup lists and checks them.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Minimum line coverage (%) for `just cov` and CI; raise it as coverage grows, never lower it.
coverage_threshold := "34"

# Cargo through scripts/env.sh, which finds rustup (keg-only in Homebrew).
cargo := "source scripts/env.sh && cargo"

default:
    @just --list

# Check the tools and install the git hooks.
setup:
    bin/setup

# Format Rust and TOML.
fmt:
    {{ cargo }} fmt --all
    taplo fmt

# Formatting check, as CI.
fmt-check:
    {{ cargo }} fmt --all -- --check
    taplo fmt --check

# clippy (warnings are errors), spelling, unused dependencies.
lint:
    {{ cargo }} clippy --workspace --all-targets --all-features --locked -- -D warnings
    typos
    @if command -v cargo-machete >/dev/null; then cargo machete; else echo "cargo-machete not installed; skipped (CI runs it)"; fi

# Tests with nextest, plus doctests (nextest skips them).
test:
    {{ cargo }} nextest run --workspace --all-features --locked
    {{ cargo }} test --doc --workspace --locked

# Coverage summary; fails under the threshold. `just cov --html` for a report.
cov *args:
    {{ cargo }} llvm-cov nextest --workspace --locked --fail-under-lines {{ coverage_threshold }} {{ args }}

# Builds with the MSRV (`rust-version` in Cargo.toml), as the CI msrv job does.
msrv:
    source scripts/env.sh && msrv="$(sed -n 's/^rust-version = "\(.*\)"/\1/p' Cargo.toml)" \
      && rustup toolchain install "$msrv" --profile minimal \
      && cargo +"$msrv" check --workspace --all-targets --locked

# RustSec advisories for every crate in Cargo.lock.
audit:
    {{ cargo }} audit --deny warnings

# Advisories, licenses, bans and sources (deny.toml).
deny:
    {{ cargo }} deny check

# Secrets in the git history.
secrets:
    gitleaks git --no-banner --redact

# GitHub Actions workflows.
actions-lint:
    actionlint
    zizmor --offline .github

# Everything CI runs.
check: fmt-check lint test msrv deny cov secrets actions-lint

# CHANGELOG.md preview of the next version.
changelog:
    bin/changelog

# Cut a release (changelog, notes, commit, tag; no push).
release part="auto":
    bin/release {{ part }}
