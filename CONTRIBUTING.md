# Contributing

## Setup

```sh
brew install rustup && rustup-init   # the toolchain in rust-toolchain.toml is installed on first use
bin/setup                            # checks the tools (prints a brew command for missing ones), installs git hooks
```

## Workflow

- `just check` runs everything CI runs: formatting (rustfmt, taplo), clippy with warnings as errors,
  typos, unused dependencies, tests (nextest + doctests), the MSRV build, cargo-deny, coverage,
  secret scanning and the workflow linters. `just --list` shows the single steps.
- Git hooks (lefthook) run the fast checks on commit and clippy, tests and cargo-deny on push. Skip them
  only deliberately (`--no-verify`); CI runs everything again and `ci-ok` must be green to merge.
- Project conventions — architecture, UI rules, security — are in [`CLAUDE.md`](CLAUDE.md).

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org), checked by the commit-msg hook:

```
<type>[(scope)][!]: <imperative description>

feat(git): add a Fetch button to the Branches popup
fix: keep the scroll position after ⌘D
refactor(lsp)!: replace the request API     ← breaking (or a "BREAKING CHANGE:" footer)
```

Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `chore`, `ci`, `build`, `revert`. The subjects
become `CHANGELOG.md` (`bin/changelog`); `feat` / `fix` / breaking changes decide the next version
(`bin/release`).

## Lints

Lints live in the root `Cargo.toml` (`[workspace.lints]`). Fix what they flag; when an `#[allow]` is the
right call, put it on the smallest item with a comment explaining why.
