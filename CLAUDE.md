# Power IDEA — project rules

A fast, language-agnostic desktop IDE in Rust (GPU UI via `gpui-kit`) with
JetBrains ergonomics: keymap, Search Everywhere, Commit tool window, terminal,
and an MCP server for AI agents.

Progress, backlog and known gaps live in `PROGRESS.md` — read it first and
update it when a feature lands.

## Commands

```sh
scripts/run.sh [project-dir]     # run the IDE (debug) on a project
scripts/check.sh [--live]        # fmt + clippy -D warnings + test; must be clean before a change is done
scripts/build.sh [--no-open]     # release "Power IDEA.app" (scripts/bundle-macos.sh) and open it
scripts/dmg.sh [--install]       # dist/*.dmg installer; --install also updates /Applications
scripts/install.sh [file.dmg]    # install / update /Applications from a disk image
scripts/version.sh [--json]      # the app version, computed from git tags (never edit versions by hand)
bin/changelog                    # regenerate CHANGELOG.md (git-cliff) and preview [Unreleased]
bin/release [major|minor|patch|auto]   # changelog + release notes + commit + annotated tag (no push)
bin/setup                        # check tools, install git hooks (lefthook.yml)
just check                       # everything CI runs (fmt, lint, test, deny, …); see justfile
```

Versions come from annotated `v*` tags: `1.5.0 (412)` on a tag, `1.5.0-dev.3+abc1234[.dirty]`
after it (`scripts/version.sh`, embedded by `crates/app/build.rs`). The `version` in Cargo.toml
is a placeholder.

The scripts find rustup themselves (`scripts/env.sh`; Homebrew's rustup is keg-only).

User config lives in `~/Library/Application Support/power-idea/`
(`config.toml`, `keymap.toml`, `mcp-token`). Project config lives in
`<project>/.power-idea/settings.toml`.

## Architecture

```
crates/
  search/  file index, fuzzy matching, find-in-files        (no UI)
  vcs/     git via the system `git` CLI                        (no UI)
  mcp/     MCP server: JSON-RPC over local HTTP               (no UI)
  lsp/     LSP client over stdio (live test: cargo test -p lsp -- --ignored)
  db/      PostgreSQL/MySQL/SQLite (sqlx) and Redis; passwords in macOS Keychain
  run/     run configurations (run.toml + detection), file:line links in output
  docker/  Docker Engine API over the unix socket, events, project resolver
  insights/ Claude Code transcripts → metrics → scores → tips; SQLite store (Session Insights)
  dap/     Debug Adapter Protocol client + adapter plans (live test: cargo test -p dap -- --ignored)
  extensions/ extension manifests, protocol and processes: JSON-RPC over stdio (docs/extensions.md)
  claude_integration/ Claude Code settings: rules, skills, resolver, launcher, installer, hook bridge
                      (docs/claude-code-integration.md)
  error_journal/ recent errors (panics, log::error!, report()) → errors.json; badge on the settings gear
  app/     the gpui application; the only crate that depends on gpui-kit
    workspace.rs        Workspace entity: state + layout composition only
    workspace/*.rs      one concern per module (editors, files, git, git_panel,
                        sidebar, chrome, terminal_panel, unsaved, mcp_tools,
                        navigation, code_nav, preview_view, run_configs, debug/,
                        session_state, action_handlers, git_log/)
    language.rs         language servers per project + editor LSP providers
    database/           Database tool window: data sources, consoles, results grid
    projects.rs         opening projects (windows grouped as native tabs)
    file_icons.rs       file-type icon + tint mapping
    inbox.rs            background-thread → UI event queue
    terminal/           session (PTY), input (keys→bytes), colors, view
    palette/            modal pickers (Search Everywhere, Go to File, usages, symbols, …)
    preview.rs          Markdown/SVG/image preview rules
    session.rs          per-project workspace state (.power-idea/workspace.toml)
    welcome/            Welcome window: recent projects, New Project
    usages.rs           Find Usages grouping (UI in workspace/usages_panel.rs)
    recovery.rs         unsaved text kept across an uncancellable quit
    menus.rs            macOS menu bar (every menu item is an action)
    watcher.rs          classifies file system events (project folder watching)
    settings.rs         Settings tab (Cmd+,)
    actions.rs          every action + the default JetBrains keymap
    mcp_bridge.rs       MCP server thread ⇄ UI thread
    extension_registry.rs installed extensions, their Find Action entries and key bindings
                        (processes per window: workspace/extensions.rs, extension_requests.rs)
    style.rs            shared visual helpers (glow, icon_button, …)
    themes/*.json       the only place for UI and syntax colors (neo.json, graphite.json)
```

Rules:
- **Logic lives in UI-free crates or pure functions** and has unit tests. The
  `app` crate wires state to views; if a piece of `app` logic can be a pure
  function (e.g. `editor::toggle_line_comment`), make it one and test it.
- **Keep files focused.** When a file passes ~500 lines or mixes concerns,
  split it into submodules (`impl Workspace` blocks can live in any child
  module of `workspace`). Use `pub(super)` for cross-module internals; keep
  `pub` for the crate API.
- **No new dependencies without the owner's approval.** Pin versions with `=`
  in `[workspace.dependencies]`, prefer crates already in `Cargo.lock`.

## Patterns

- **Entities + actions.** UI state is a gpui `Entity`; user intents are
  `actions!` routed through `workspace/action_handlers.rs`. Context menus
  dispatch typed actions (`FileAction { op, path }`) with
  `PopupMenu::action_context(workspace.focus_handle)`.
- **Never block the UI thread.** Disk walks, git, search and network run in
  `cx.background_spawn`; results come back via `this.update(cx, …)`. Keep the
  returned `Task` or `.detach()` it deliberately.
- **Errors:** `anyhow::Result` with `.context(..)` in crates; in the UI,
  surface failures with `window.push_notification(Notification::error(..))`.
  Don't `unwrap` on I/O or user input. `eprintln!` is for startup/logging only.
- **Destructive actions ask first** (rollback, delete → moves to Trash,
  closing unsaved tabs/windows).
- **Composition over copy-paste:** extract a helper (`style::icon_button`,
  `settings::server_field`, `Workspace::run_git`) once a pattern repeats.
- **Paths:** tools and menus use paths relative to the project root; validate
  that user/agent-supplied paths stay inside the project.

## Commits

- **Commit after every finished change, without asking**: once `scripts/check.sh`
  is clean and `PROGRESS.md` is updated, commit on the current branch (`main`).
- One commit per feature or fix, in **Conventional Commits** form (the commit-msg hook checks it):
  `feat(scope): add …`, `fix: …`, `perf`, `refactor`, `docs`, `test`, `chore`, `ci`, `build`;
  `!` or a `BREAKING CHANGE:` footer for breaking changes. English, imperative description, ending with
  the session's attribution trailer. `CHANGELOG.md` is generated from these subjects (`bin/changelog`).
- Commit only files that belong to the change; look at unknown files before adding them.
- `git push`, force-push and history rewrites still need the owner's explicit request.

## Code style

- Code, comments, identifiers, commits: English. `rustfmt.toml` sets
  `max_width = 110`; workspace lints are in the root `Cargo.toml`.
- Comments explain *why*, not *what*. Module docs (`//!`) state the module's
  responsibility in one or two lines.
- Small functions with one job; early returns (`let … else`) over nesting.
- `unsafe` is denied; the one exception must carry a `// SAFETY:` comment.

## UI and design ("Neo")

- Colors come from `cx.theme()` only; add or change colors in
  `crates/app/themes/neo.json` (dark + light) and `graphite.json` (dark). Hard-coded colors are allowed
  only for the terminal ANSI palette.
- One accent (`theme.primary`), used sparingly: active indicators, focus,
  primary buttons. Glow via `style::glow`, never stronger than ~0.8 in chrome.
- Keyboard first: every feature gets an action, a JetBrains-compatible
  default binding where one exists, and an entry in Find Action
  (`actions::palette_actions`).

## Security

- The MCP server binds to 127.0.0.1, requires the bearer token, rejects
  non-local `Origin`s, and exposes read-only tools plus UI navigation. Never
  add tools that write files, run commands, or leak the token.
- Never read, print or commit secrets (`mcp-token`, `.env*`, keys).
- Git runs with `GIT_TERMINAL_PROMPT=0`; never force-push or rewrite history
  from the IDE without an explicit user action.
