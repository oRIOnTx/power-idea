---
name: conventional-commits
description: Write Conventional Commits messages and user-facing release notes. Use when committing, preparing a release or writing a changelog entry.
---

# Conventional commits and release notes

Commit subject: `<type>[(scope)][!]: <imperative description>`, at most ~72 characters.

- Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `chore`, `ci`, `build`, `revert`.
- `!` after the type/scope (or a `BREAKING CHANGE:` footer) marks a breaking change.
- The body explains *why*, wrapped at 72 columns; reference issues in the footer.
- One logical change per commit; never mix formatting with behavior changes.

Release notes are for users: what they can now do or what got better, no hashes,
ticket ids or internal module names, at most six bullets.
