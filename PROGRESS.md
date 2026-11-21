# Power IDEA — progress

Updated: 2026-11-22. Last commit: `0990e51`. Everything below marked *uncommitted*
is in the working tree only.

## Done

### Core (committed)
- [x] Cargo workspace: `search`, `vcs`, `mcp`, `app` crates; project rules (`CLAUDE.md`) and lints
- [x] Editor with tree-sitter highlighting (~35 languages), tabs, JetBrains line operations
- [x] Project tree, Search Everywhere (⇧⇧), Go to File (⌘⇧O), Recent Files (⌘E), Find in Files (⌘⇧F), Find Action (⌘⇧A), Go to Line (⌘L)
- [x] Integrated terminal (⌥F12, tabs, copy/paste, selection, scrollback)
- [x] Git: status colors, Commit tool window (⌘0), side-by-side diffs, push/pull/fetch, branches, rollback
- [x] MCP server for Claude Code (13 tools, token in Keychain-like file, localhost only)
- [x] Neo theme (dark/light), app icon, `scripts/bundle-macos.sh`
- [x] Settings tab (⌘,): appearance, editor, terminal, languages, MCP, keymap, project (`.power-idea/settings.toml`)
- [x] Activity bar, file/tab context menus, unsaved-changes guard on tab/window close

### Uncommitted
- [x] LSP client (`crates/lsp`): completion, hover, diagnostics, ⌘+Click / ⌘B go to definition, ⌘[ / ⌘] history
- [x] LSP status chips (starting / indexing % / ready / fallback / failed), preload on project open, failure notifications with cause
- [x] Ruby fallback mode (minimal bundle) + "Fix in Terminal"
- [x] File-type icons and folder tints; tree indentation fix
- [x] Database tool window (`crates/db`): PostgreSQL, MySQL, SQLite, Redis; schema tree; SQL/Redis consoles (⌘↩); passwords in macOS Keychain; data sources in `.power-idea/databases.toml`
- [x] Projects open as native macOS window tabs ("This Window / New Tab"); full-screen title bar padding
- [x] Bundle code signing + `scripts/create-signing-cert.sh`; larger icons; tab padding fix
- [x] Crash fix: ⌘+Click re-entrancy; panic log at `~/Library/Logs/Power IDEA/panic.log`

## In progress
- [ ] **Git Log tool window (RubyMine-style, ⌘9)**
  - [x] Commit graph layout (`vcs/src/graph.rs`, tested)
  - [x] History with ref labels, commit details, file diffs (`vcs/src/history.rs`, parsing tested)
  - [x] Ops: cherry-pick, revert, reset soft/mixed/hard, checkout revision, branch/tag at commit, merge, rebase, rename/delete branch, push branch, continue/abort (written, not yet tested)
  - [ ] Branch list with upstream ahead/behind (`Branch` struct needs upstream fields)
  - [ ] UI: branches panel (HEAD / Local / Remote, grouped by prefix), log with graph, details pane, context menus, conflict banner

## Backlog (agreed order)
1. Commit the uncommitted work
2. Finish the Git Log tool window
3. Preview for Markdown, images, SVG (Markdown and SVG stay editable)
4. Persist workspace state in `.power-idea/` (open tabs, expanded folders, panel layout)
5. Navigation: Find Usages (⌥F7), Go to Class/Symbol (⌘O / ⌘⌥O — ⌘O currently opens a project), File Structure (⌘F12), Rename (⇧F6), Reformat (⌥⌘L)

## Known gaps
- Database: no inline data editing, no table/column completion in consoles, no export, no TLS for Redis
- Terminal: no mouse reporting for TUI apps, no cursor blink
- Tree and search miss files created outside the IDE until reopen (needs `notify`, not yet approved)
- ⌘Q quits without the unsaved-changes prompt
- PostgreSQL/MySQL/Redis drivers verified only by unit tests (no local servers); SQLite verified end to end
- The user's `lms` project needs `brew install libpq` + `bundle install` for full Ruby LSP (fallback mode works meanwhile)
