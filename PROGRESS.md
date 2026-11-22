# Power IDEA — progress

Updated: 2026-11-22. Last commit: see `git log`. Everything below marked *uncommitted*
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

### Committed 2026-11-22 (`f2fb67a`…`bda932c`)
- [x] LSP client (`crates/lsp`): completion, hover, diagnostics, ⌘+Click / ⌘B go to definition, ⌘[ / ⌘] history
- [x] LSP status chips (starting / indexing % / ready / fallback / failed), preload on project open, failure notifications with cause
- [x] Ruby fallback mode (minimal bundle) + "Fix in Terminal"
- [x] File-type icons and folder tints; tree indentation fix
- [x] Database tool window (`crates/db`): PostgreSQL, MySQL, SQLite, Redis; schema tree; SQL/Redis consoles (⌘↩); passwords in macOS Keychain; data sources in `.power-idea/databases.toml`
- [x] Projects open as native macOS window tabs ("This Window / New Tab"); full-screen title bar padding
- [x] Bundle code signing + `scripts/create-signing-cert.sh`; larger icons; tab padding fix
- [x] Crash fix: ⌘+Click re-entrancy; panic log at `~/Library/Logs/Power IDEA/panic.log`

### Committed 2026-11-22 (second batch)
- [x] **Git Log tool window** (⌘9, bottom panel): graph, branches panel with ahead/behind, filter, details → diffs,
      commit/branch context menus (cherry-pick, revert, reset, merge, rebase, rename, delete…), conflict banner
- [x] **Previews**: Markdown and SVG get editor / split / preview modes (stay editable, live update,
      local images in Markdown resolve relative to the file); PNG/JPG/GIF/WebP/… open in an image viewer (Fit / 100%)
- [x] **Workspace state** in `.power-idea/workspace.toml` (auto git-ignored): open tabs with cursors and
      preview modes, active tab, expanded folders, recent files, sidebar panel, Git Log; saved every 2 s and on close
- [x] **Code navigation via LSP**: Find Usages (⌥F7), Go to Class (⌘O), Go to Symbol (⌘⌥O),
      File Structure (⌘F12), Rename (⇧F6, updates open editors, rewrites other project files),
      Reformat Code (⌥⌘L). Open Project lost ⌘O (title bar button / Find Action)
- [ ] Previews, Git Log and navigation UI not clicked through by hand (no UI automation here);
      LSP requests verified against real ruby-lsp (`cargo test -p lsp -- --ignored`)

### Committed 2026-11-22 (third batch)
- [x] Self-contained scripts: `scripts/run.sh`, `check.sh`, `build.sh`, `bump-version.sh` (find rustup themselves)
- [x] Welcome screen (JetBrains style) after closing the last project: recent projects with search, New Project
      (folder + optional `git init`), Open
- [x] ⌘Q asks about unsaved files in every project window (Save all / Don't Save / Cancel)
- [x] Version `0.2.0 (build N, hash[+])`: build number = git commit count, `+` = uncommitted changes;
      shown on the Welcome screen, in `panic.log` and as `CFBundleVersion`

- [x] File watching (`notify`, FSEvents): tree + search index refresh, clean editors reload, warning for edited ones;
      `.git`, `target`, `node_modules`, `tmp`, `log`, `.power-idea` and excluded paths are ignored
- [x] macOS menu bar (JetBrains layout): Power IDEA (About, Settings, Hide, Quit), File (New Project, Open,
      Recent, Save, Close Tab, Close Project), Edit, View, Navigate, Code, Git, Window, Help; the title-bar
      close button moved to File → Close Project (⌘⇧W)
- [x] Opening a project from Recent Projects / Welcome asks This Window / New Tab / Cancel again (the dialog
      could not open inside the caller's window update and silently fell back to a new window); project tabs
      are drawn under the title bar because the custom title bar hides the native macOS tab strip
- [x] Find Usages tool window (⌥F7, bottom panel): usages grouped by file (project files first), match
      highlighted, collapsible files, ↑/↓ select, Enter/F4 or click jumps; a single usage jumps directly
- [x] Database consoles: completion of tables, columns (`table.` → its columns) and SQL keywords;
      Export CSV (read-only queries are re-run to export every row, up to 1M); editing cells of a
      single-table SELECT that includes the primary key: double-click → value / NULL, pending edits
      highlighted, Submit runs one UPDATE per cell and reloads, Revert drops them
- [x] Terminal: mouse reporting for full-screen apps (xterm 1000/1002/1003, X10/UTF-8/SGR encodings;
      Shift+drag still selects), wheel → arrow keys in `less`/`man` (alternate scroll), blinking cursor
- [x] Git Log polish: right-click selects the commit, dates in the local time zone (details show the time),
      filter terms `author:name` and `path:dir/file` (path is filtered by git, graph hidden), "Load more"
      pages of 2000 commits
- [x] Database: add rows (+ Row, filled via the cell editor) and delete rows (− Row toggles); Submit runs
      DELETE → UPDATE → INSERT; completion resolves table aliases (`FROM users u` → `u.` lists users' columns)

## Backlog
Agreed backlog is done. Candidates:
Database gaps below.

## Known gaps
- `db` test `sqlite_queries_schema_and_limits` failed once; temp DB names now use a counter instead of the clock
- Navigation: Rename does not save changed open editors; ruby-lsp formats only with RuboCop installed;
  Go to Symbol lists library symbols after project ones
- Git Log: no date-range filter
- Database: no TLS for Redis; submitted statements are not wrapped in one transaction
- Quitting from the Dock icon bypasses the unsaved-changes prompt (menu Quit and ⌘Q ask)
- PostgreSQL/MySQL/Redis drivers verified only by unit tests (no local servers); SQLite verified end to end
- The user's `lms` project needs `brew install libpq` + `bundle install` for full Ruby LSP (fallback mode works meanwhile)
