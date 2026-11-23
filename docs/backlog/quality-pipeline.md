# Backlog: strict code quality, security and CI/CD pipeline

Spec as given by the owner (2026-11-23), kept verbatim. Not started.

Notes for this repo: a Cargo workspace (`crates/*`), macOS desktop app, no CI yet, edition 2024. Every tool
listed (lefthook, cargo-deny, nextest, llvm-cov, taplo, typos, gitleaks, git-cliff, …) is a new install and
needs the owner's approval first. `unsafe_code` is `deny` with one documented exception, so it cannot become
`forbid`. Overlaps with `versioning-and-changelogs.md` (git-cliff, Conventional Commits, release flow):
do them together.

---

Set up a strict, production-grade code quality, security and CI/CD pipeline for this Rust project. Use established, well-maintained tools only — no custom scripts where a standard tool exists. Same checks must run locally (hooks) and in CI, so "green locally" = "green in CI".

First inspect the repo: workspace or single crate, binary/library/Tauri app, target OSes, existing CI, current Rust edition and `rust-version`. Tell me what you found and what you plan to add before changing files.

## 1. Toolchain & formatting
- `rust-toolchain.toml` pinning stable + components `rustfmt`, `clippy`, `llvm-tools-preview`.
- Set `rust-version` (MSRV) in Cargo.toml; verify it with `cargo-msrv verify` (CI job builds on MSRV).
- `rustfmt.toml` (stable options only; `edition` matching Cargo.toml).
- `taplo` for formatting/linting all TOML files (`.taplo.toml`).
- `.editorconfig`.

## 2. Linting
- Put lint config in Cargo.toml `[lints]` / `[workspace.lints]` (not scattered `#![deny]`):
  - rust: `unsafe_code = "forbid"` (or `deny` with justification if the project needs unsafe), `missing_debug_implementations`, `unused_qualifications`, `rust_2018_idioms`.
  - clippy: `all` + `pedantic` = warn, plus selected restriction lints: `unwrap_used`, `expect_used` (allowed in tests via `clippy.toml` `allow-unwrap-in-tests`), `panic`, `todo`, `dbg_macro`, `print_stdout`/`print_stderr` (unless CLI), `indexing_slicing`. Allow noisy pedantic ones with a comment why (e.g. `module_name_repetitions`, `must_use_candidate`).
- CI runs `cargo clippy --all-targets --all-features --locked -- -D warnings`.
- `typos` (crate-ci/typos) for spelling in code and docs, with `_typos.toml` for false positives.
- `cargo machete` for unused dependencies.
- If it's a library: `cargo semver-checks` on PRs; `cargo doc --no-deps` with `RUSTDOCFLAGS="-D warnings"`.

## 3. Security
- `cargo-deny` with `deny.toml`:
  - advisories: RustSec DB, deny vulnerabilities and unmaintained/yanked crates;
  - licenses: allow-list (MIT, Apache-2.0, BSD-2/3, ISC, Unicode-3.0, Zlib, MPL-2.0 — adjust to what's actually in the tree; explain any additions);
  - bans: deny duplicate major versions where practical (warn otherwise), deny wildcard versions;
  - sources: only crates.io unless I've whitelisted a git source.
- `cargo audit` as a scheduled daily CI job (catches new advisories on unchanged code) — opens/updates an issue on failure.
- `cargo geiger` report (non-blocking) to track unsafe in dependencies.
- Secret scanning: `gitleaks` in pre-commit and CI (with `.gitleaks.toml` allow-list if needed).
- GitHub Actions hardening: pin every third-party action to a full commit SHA (with version comment), `permissions: contents: read` at workflow level and minimal per-job elevation, `persist-credentials: false` on checkout, lint workflows with `zizmor` and `actionlint`.
- CodeQL for Rust (GitHub code scanning) if the repo is on GitHub; OpenSSF Scorecard workflow (weekly) with results to code scanning.
- `SECURITY.md` with a vulnerability reporting policy.

## 4. Tests & coverage
- `cargo nextest run --all-features --locked` (+ `cargo test --doc` since nextest skips doctests).
- Coverage with `cargo llvm-cov nextest` → lcov artifact; print summary in the job; fail if coverage drops below a threshold I'll set (start with current value, rounded down).
- If there's any unsafe code: a `cargo miri test` job (nightly, allowed to be slow, runs on schedule + PRs touching unsafe).

## 5. Git hooks (local)
Use `lefthook` (`lefthook.yml`) — committed config, installed via `lefthook install` documented in README and a `just setup` task.
- pre-commit (fast, staged files only, parallel): `cargo fmt --check`, `taplo fmt --check`, `typos`, `gitleaks protect --staged`.
- commit-msg: enforce Conventional Commits (`feat|fix|perf|refactor|docs|test|chore|ci|build`, `!` for breaking) — use `committed` (crate-ci) or `cocogitto` (`cog verify`).
- pre-push: `cargo clippy ... -D warnings`, `cargo nextest run`, `cargo deny check`.
Hooks must be skippable only explicitly (`--no-verify`) and CI must re-run everything.

## 6. Task runner
`justfile` with: `setup`, `fmt`, `lint`, `test`, `cov`, `audit`, `deny`, `check` (= everything CI runs), `release`. README "Development" section lists them.

## 7. CI (GitHub Actions)
`.github/workflows/ci.yml` on PR + push to main, with `concurrency` cancel-in-progress and `Swatinem/rust-cache`:
- `fmt` (rustfmt + taplo), `lint` (clippy, typos, machete), `test` (nextest + doctests, matrix: ubuntu/macos/windows — only the OSes this app targets), `msrv`, `deny`, `coverage`, `docs` (if lib), `actions-lint` (zizmor + actionlint).
- All jobs use `--locked`; `Cargo.lock` committed.
- A final `ci-ok` job that depends on all others — the single required status check for branch protection.
Separate workflows: `audit.yml` (daily cron), `codeql.yml`, `scorecard.yml`.

## 8. Dependencies
`.github/dependabot.yml` for `cargo` and `github-actions` (weekly, grouped minor/patch updates) — or Renovate if already used. Dependency PRs must pass the same CI.

## 9. Release / CD
- Versioning from git tags + Conventional Commits; changelog via `git-cliff` (`cliff.toml`, Keep a Changelog style).
- Use `release-plz` (release PR with version bump + CHANGELOG, tag on merge) — or `cargo-dist` for building and publishing binaries/installers for target OSes if this is an app. For a Tauri app use `tauri-apps/tauri-action` instead.
- Release artifacts: checksums (SHA256), build provenance attestation (`actions/attest-build-provenance`), optional SBOM via `cargo cyclonedx`.
- Never publish to crates.io or push tags without my confirmation in this session; in CI publishing only happens on tag/release-PR merge.

## 10. Repo hygiene
`CODEOWNERS`, PR template with checklist, issue templates, `CONTRIBUTING.md` (commit format, `just check`), `.gitignore` review. Tell me the branch protection settings to enable (require `ci-ok`, require linear history / signed commits optional, dismiss stale reviews).

## 11. Finish
- Fix (or explicitly `#[allow]` with a reason) everything the new lints flag — show me a summary of what changed in source code separately from config.
- Run `just check` locally and show the result.
- List every file created/changed and any tool I need to install locally (`cargo install --locked ...` or `cargo binstall`).
- Don't push, open PRs or create tags — ask me first.
