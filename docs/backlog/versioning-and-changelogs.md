# Backlog: automatic versioning and changelogs

Spec as given by the owner (2026-11-23), kept verbatim. Not started.

Notes for this repo: it is a Rust/gpui macOS app, so of section 1 only the shared `scripts/version.sh`
applies (injected through the build / `scripts/bundle-macos.sh` Info.plist, not Xcode, Rails, Vite or Expo).
It replaces today's `scripts/bump-version.sh` + `0.2.0 (build N, hash[+])` scheme. The Conventional
Commits rule changes the commit style in `CLAUDE.md` ("Add …" → "feat: add …"). `git-cliff` is a new tool
and needs the owner's approval before installing.

---

Set up automatic versioning and changelogs for this project. Goal: during development I never edit version numbers or CHANGELOG by hand. Git is the single source of truth; the app always shows the exact version it was built from; changelogs are generated from commits.

## Version rules
- Marketing version = latest annotated tag matching `v*` (`v1.5.0` → `1.5.0`). No tags yet → `0.1.0` and tell me to create the first tag.
- Build number = `git rev-list --count HEAD` (monotonic, identical on every machine for the same commit).
- Dev build (commits after last tag): `1.5.0-dev.3+abc1234` (3 = commits since tag from `git describe --tags --long`, abc1234 = short SHA), plus `.dirty` if the tree has uncommitted changes.
- Release build (HEAD exactly on a tag): clean `1.5.0 (412)`.
- No git at build time (Docker, CI shallow clone) → use env vars `APP_VERSION` / `APP_BUILD`; never fail the build because of versioning.

## 1. Detect the stack and inject the version at build time
Inspect the repo, tell me which stack(s) you found, then implement the matching option(s). Never commit generated version values.
- **Xcode (macOS/iOS):** add a Run Script build phase (after "Copy Bundle Resources") calling `scripts/version.sh`, which writes `CFBundleShortVersionString` and `CFBundleVersion` into the built product's Info.plist via PlistBuddy (`$TARGET_BUILD_DIR/$INFOPLIST_PATH`), not the source plist. Disable "Based on dependency analysis" for it. Leave `MARKETING_VERSION`/`CURRENT_PROJECT_VERSION` in the project as placeholders.
- **Rails:** `config/initializers/app_version.rb` exposing `AppVersion.version`, `.build`, `.sha`; reads env vars first, then git, then a `REVISION` file. For Kamal/Docker: pass `APP_VERSION`, `APP_BUILD`, `GIT_SHA` as build args in `config/deploy.yml` (computed by `scripts/version.sh`), write them into ENV in the Dockerfile, so production works without `.git`.
- **Vue / Vite / Node:** inject via Vite `define` (`__APP_VERSION__`, `__APP_BUILD__`) computed in `vite.config` from `scripts/version.sh`; add a TS declaration.
- **Expo:** `app.config.js` reads version/build from the script; set `ios.buildNumber` and `android.versionCode` from the build number.

`scripts/version.sh` is the one shared script: prints `VERSION`, `BUILD`, `SHA`, `FULL` (as shell-sourceable lines, or `--json`). Every integration calls it; no duplicated logic.

## 2. Show it in the app
Add the version where users/testers can see it: About window / settings / footer / `/version` JSON endpoint (Rails) — whichever fits this app. Dev builds show the full `-dev.N+sha` string; release builds show `1.5.0 (412)`. Add a tooltip or copy button with the full string for bug reports.

## 3. Commit convention (needed for changelogs)
- Use Conventional Commits: `feat:`, `fix:`, `perf:`, `refactor:`, `docs:`, `chore:`, `test:`, `ci:`; `!` or `BREAKING CHANGE:` footer for breaking changes.
- Add a lightweight `commit-msg` git hook (in `.githooks/`, enabled via `git config core.hooksPath .githooks` in a `bin/setup` step) that rejects non-conforming messages with a helpful hint. Allow merge and revert commits.

## 4. Changelogs
- Use `git-cliff` (config in `cliff.toml`) — if it's not installed, add install instructions to README and fall back to a small script in `scripts/` that produces the same output.
- Format: Keep a Changelog. Sections: Added (feat), Fixed (fix), Changed (refactor/perf), Removed, Breaking. Skip docs/chore/test/ci and version-bump commits. Each line rewritten as a readable sentence, with short SHA link.
- `CHANGELOG.md` has an `## [Unreleased]` section regenerated on every run of `bin/changelog` (so I can preview what's going into the next version at any time).

## 5. Release command
Create `bin/release [major|minor|patch|auto]` (default `auto`):
1. Refuse if the working tree is dirty or not on the main branch (print why).
2. `auto` picks the bump from commits since the last tag: breaking → major (minor while on 0.x), feat → minor, otherwise patch.
3. Print the plan: `1.4.2 → 1.5.0`, commit list grouped by type, and the changelog section — then ask `Proceed? [y/N]`.
4. Generate the new `## [1.5.0] - YYYY-MM-DD` section in CHANGELOG.md, clear Unreleased.
5. Also write `release-notes/v1.5.0.md`: short user-facing notes (benefits, not internals; no SHAs or ticket IDs; max ~6 bullets) for GitHub Releases / App Store "What's New".
6. Commit `chore(release): v1.5.0` and create annotated tag `v1.5.0`.
7. Do NOT push. Print the next commands (`git push --follow-tags`, and `gh release create v1.5.0 --notes-file release-notes/v1.5.0.md` if `gh` is available).

## 6. Optional CI
If the repo has GitHub Actions: add a workflow on tag push `v*` that builds with `fetch-depth: 0` (so describe/count work) and creates the GitHub Release from `release-notes/<tag>.md`. Don't add CI if none exists — just mention it.

## 7. Finish
- Update README with a short "Versioning & releases" section (commit format, `bin/changelog`, `bin/release`).
- Run everything once: show me the current computed version, build a dev build and confirm the app shows it, run `bin/changelog` and show the Unreleased section. Don't create a real tag — ask me first.
- List every file you created or changed.
