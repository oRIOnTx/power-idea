# Power IDEA — progress

Updated: 2026-11-23. Last commit: see `git log`. Everything below marked *uncommitted*
is in the working tree only.

## Done

### Core (committed)
- [x] Cargo workspace: `search`, `vcs`, `mcp`, `app` crates; project rules (`CLAUDE.md`) and lints
- [x] Editor with tree-sitter highlighting (~35 languages), tabs, JetBrains line operations
- [x] Project tree, Search Everywhere (⇧⇧), Go to File (⌘⇧O), Recent Files (⌘E), Find in Files (⌘⇧F), Find Action (⌘⇧A), Go to Line (⌘L)
- [x] Integrated terminal (⌥F12, tabs, copy/paste, selection, scrollback)
- [x] Bottom tool windows span the full window width under the Project tree (Settings → Terminal → Full window width; on by default)
- [x] `.power-idea` folder hidden in the Project tree; Settings → Appearance → Project tree → Show .power-idea folder
- [x] Settings → System page: "Open projects in" and Confirmations (exit / close project) moved out of Appearance
- [x] **LightEdit / quick editor**: files opened from Finder (Open With, Dock icon), `power-idea <files…>` or File → Open…
      (now files or folders) open in the project that contains them, else in a LightEdit window (no index, git,
      language servers or saved state, nothing written next to the file). The bundle registers ~190 text
      extensions (`crates/app/text-extensions.txt`) plus folders; more extensions map to highlighters (ini/cfg → TOML, xml/plist → HTML, …)
- [x] Git: status colors, Commit tool window (⌘0), side-by-side diffs, push/pull/fetch, branches, rollback
- [x] MCP server for Claude Code (17 tools, token in Keychain-like file, localhost only)
- [x] Neo theme (dark/light), app icon, `scripts/bundle-macos.sh`
- [x] Settings tab (⌘,): appearance, editor, terminal, languages, MCP, keymap, project (`.power-idea/settings.toml`)
- [x] Activity bar, file/tab context menus, unsaved-changes guard on tab/window close

### Committed 2026-11-22 (`f2fb67a`…`bda932c`)
- [x] LSP client (`crates/lsp`): completion, hover, diagnostics, ⌘+Click / ⌘B go to definition, ⌘[ / ⌘] history
- [x] Hover popover: one-line code blocks shown as inline code, language labels dropped (gpui-kit draws them over the code)
- [x] LSP status chips (starting / indexing % / ready / fallback / failed), preload on project open, failure notifications with cause
- [x] Ruby fallback mode (minimal bundle) + "Fix in Terminal"
- [x] File-type icons and folder tints; tree indentation fix
- [x] Database tool window (`crates/db`): PostgreSQL, MySQL, SQLite, Redis; schema tree; SQL/Redis consoles (⌘↩); passwords in macOS Keychain; data sources in `.power-idea/databases.toml`
- [x] Projects open as native macOS window tabs ("This Window / New Tab"); full-screen title bar padding
- [x] Bundle code signing + `scripts/create-signing-cert.sh` (run automatically on the first build); larger icons; tab padding fix
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
- [x] Installer: `scripts/dmg.sh` (drag-to-Applications disk image in `dist/`, signed with the local
      certificate when present) and `scripts/install.sh` (install / update, quits the running app gracefully)
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
- [x] Database Submit runs in one transaction (all or nothing)
- [x] Quit that cannot be cancelled (Dock → Quit, logout) keeps unsaved editor text in
      `~/Library/Application Support/power-idea/recovery/` and restores it as unsaved edits on next open
      ("Don't Save" on ⌘Q skips this)
- [x] Redis TLS (`rediss://` when "Require TLS" is set; redis `tls-rustls` feature, no new crates)
- [x] Git Log date filter: `after:YYYY-MM-DD` / `before:YYYY-MM-DD`, applied by git like `path:`
- [x] MCP tools backed by language servers (read-only): `get_diagnostics`, `find_symbol`, `find_usages`,
      `get_file_structure`
- [x] Replace in Files (⌘⇧R): find/replace inputs, Match case / Words, live preview of every matching line with
      the replacement, Replace All after a confirming second click; open files change in their editors
      (unsaved), others on disk. `search::{match_ranges, replace_all}` share Find in Files' matching rules
- [x] Run configurations, any language (`crates/run`): `.power-idea/run.toml` plus detected ones (Rails server/
      console/tests, RSpec, cargo, npm/yarn/pnpm/bun scripts, go, pytest, Django, mix, make targets). Each runs
      in its own terminal tab through the login shell (colors, input, pry/byebug work), the tab stays with the
      exit code; title-bar widget (picker ▶ ■), Run ⌃R, Run… ⌃⌥R, Stop ⌘F2, Run menu, Edit Configurations
- [x] Run configurations: JetBrains-style macros (`$FilePath$`, `$FileRelativePath$`, `$FileDir$`, `$FileName$`,
      `$FileNameWithoutExtension$`, `$ProjectDir$`; shell-quoted in `command`). Detected "Python: current file"
      for Python projects runs/debugs the open file from its folder via `.venv`/`venv` python or `python3`,
      saving it first
- [x] ⌘+click on `file:line[:col]` in any terminal opens it (Ruby, Rust, Go, Node, Python tracebacks…)
- [x] Debugging for any language via the Debug Adapter Protocol (`crates/dap`): rdbg (Ruby: debugs the run
      command itself, e.g. rails server / rspec), lldb-dap (Rust, C/C++; `cargo build` first), debugpy (Python,
      program or module), dlv (Go). Breakpoints ⌘F8 (red gutter marks), Debug ⌃D, Resume ⌥⌘R, Step Over F8 /
      Into F7 / Out ⇧F8, Pause, Stop ⌘F2; debug tool window with frames, expandable variables, console and
      expression evaluation; paused line highlighted. `[config.debug]` in run.toml; detected for Rails/RSpec,
      cargo run, go run, Django, pytest. Verified live against rdbg and lldb-dap (`cargo test -p dap -- --ignored`)
- [x] Welcome screen restyled after the README illustration (centered title, 48 px project rows, ↑/↓ + Enter)
- [x] Project tree: Cut / Copy / Paste of files and folders (Finder-style " copy" names, folders recursively,
      moves across volumes, open tabs follow moved files; Paste into the clicked folder or the file's folder)
- [x] Graphite theme: a lighter dark theme in the style of current JetBrains IDEs (editor #1E1F22, panels
      #2B2D30, blue accent, JetBrains-like syntax colors); Settings → Appearance → Theme
- [x] Docker panel (MVP + parts of stage 2–3), `crates/docker`: Engine API over the unix socket
      (no CLI per action), socket from DOCKER_HOST → docker context → Docker Desktop / OrbStack / Colima;
      project containers by compose labels + `docker compose config --format json`; live status from `/events`
      (one connection, debounced re-list); start / stop / restart / remove (confirm); logs, shell and
      `compose up -d / restart / down` in terminal tabs; clickable `:PORT` links; health colors; running-count
      badge; lazy (connects when the panel opens); "Docker is not running" + Check again
- [x] Claude Session Insights (MVP + scoring + rule tips), `crates/insights`: parses
      `~/.claude/projects/*/*.jsonl` (unknown lines skipped; usage de-duplicated per message id), pure
      metrics → 7 blocks (0–100) → Session Score; SQLite store in the config folder with incremental
      indexing by mtime/size (re-index every 2 min while the panel is open, never on the UI thread);
      sidebar panel (off by default, Settings → Insights): score + trend vs previous period (7d/30d/90d/All),
      project filter, score-per-session chart, block bars now vs before, top-3 tips with "Got it", session
      list and breakdown; "Clear all analytics data"
- [x] Insights advice: 8 data rules (+ vague first prompts, agent loops, compactions) and good-practice tips
      that fill the Tips section when few rules fire; "Ask Claude for advice" (asks first) sends the last 10
      sessions' numbers and first prompts (shortened) to `claude -p --no-session-persistence --tools ""`
      and shows the answer under the tips (not persisted)
- [x] Project tree multi-selection (⌘+click toggles, ⇧+click selects a range, arrows move it) and
      ⌘X / ⌘C / ⌘V for the selected files and folders (also Edit menu, Find Action and the context menu,
      which acts on the whole selection); pasted items become the selection
- [x] Branch widget in the title bar shows ↑ commits to push / ↓ to pull; the Branches popup starts with
      Update Project / Commit / Push (with ↑N) / Fetch and shows ↑↓ on each tracked branch; Fetch action + Git menu
- [x] Git notifications say what moved: "Pushed 3 commits to origin/main" (new branch → upstream), Update Project
      → commits, source and diff stat, Fetch → new commits per branch, new/pruned branches and how far behind
- [x] Merge conflicts (JetBrains-style): a title-bar chip "Merge: 2 conflicts" / "resolved — continue" while a
      merge/rebase/cherry-pick/revert is pending; Conflicts tab (Git → Resolve Conflicts…, the chip, clicking a
      conflicted file in Commit, or automatically when pull/merge stops on conflicts) with the conflicted files,
      per-chunk Yours | Theirs (Accept / both in either order / Undo, unchanged context around), Apply writes and
      stages the file, Accept Yours / Theirs for the whole file (deletes when that side deleted it), Open in
      Editor + Mark as Resolved for manual edits, Continue / Abort of the operation. Rebase sides are labelled
      Upstream / Your commit. `vcs::conflicts` parses markers (diff3 base too) and is tested on real merges
- [x] Terminal: slow trackpad swipes scroll (sub-line wheel deltas accumulate instead of being rounded away)
- [x] Branches popup (JetBrains style) drops down under the branch widget instead of the centered palette:
      search, Update / Commit / Push / New Branch (⌥⌘N) / Checkout Tag or Revision, Recent (from the reflog,
      `vcs::recent_branches`), Local and Remote groups (Remote folded), upstream on the right, and a submenu per
      branch (click / → / Enter: Checkout, New Branch from Here, Merge, Rebase, Push, Rename, Delete); Fetch button

- [x] Project tree root row (JetBrains style): project folder name + `~/…` location; click selects it (⌘V pastes
      into the root), double click or the chevron folds the tree; right click opens the root menu (New File /
      New Directory, Paste, Copy Path, Reveal in Finder, Open in Terminal)
- [x] Enter submits every text dialog (New File / Directory, Rename, New Worktree, New Project, Data Source)
      via `style::submit_on_enter`; fixes the abort on Enter in the name prompt (it re-entered the Workspace)
- [x] Project tree: ⌦ / ⌫ / ⌘⌫ ask and move the selected files to the Trash (multi-selection; also from the context menu);
      the confirmation lists the items (icon, name, folder) with Cancel and a red "Move to Trash"
- [x] Project tree keeps keyboard focus: a single click previews a file without moving focus to the editor
      (double click edits), the context menu returns focus to the tree, so ⌘X / ⌘C / ⌘V work on the
      selection; every row of a multi-selection is highlighted (the Tree only paints its cursor row)
- [x] Destructive confirmations (Rollback, Hard Reset, Abort, Delete Branch, Remove container / data source,
      Clear analytics) share `style::danger_confirm`: red icon, Cancel (Esc) and a red named action button
- [x] "Exit Power IDEA?" on ⌘Q and "Close project “name”?" on ⇧⌘W / the close button / a project tab, with
      "Don't ask again"; toggles in Settings → Appearance → Confirmations (`confirm_exit`,
      `confirm_close_project`). Unsaved files keep their Save / Don't Save / Cancel dialog instead
- [x] Terminal ⌘V pastes copied files and images (screenshots) as shell-escaped paths; images are saved to
      `$TMPDIR/power-idea-paste/`, so Claude Code in the terminal attaches them
- [x] Project tree ⌘V / Paste takes the system clipboard first: an image (screenshot) becomes `image.png`
      (`image 2.png`, …) in the target folder, files copied in Finder are copied in; tree Copy/Cut puts the
      paths on the system clipboard as text, so an older screenshot there no longer wins
- [x] Breakpoints are saved with the workspace state (`[breakpoints]` in `workspace.toml`) and restored on reopen
- [x] Status bar shows JetBrains-style breadcrumbs (project › folders › file, with icons) instead of the
      branch and file count; a click reveals the crumb in the project tree
- [x] Docker "Show all containers" groups containers like JetBrains Services: the project, other compose
      projects by name, then standalone containers; each group folds and shows running/total
- [x] Commit tool window: file rows are indented under their group header (Changes / Unversioned Files)
- [x] Commit tool window: long file names truncate with an ellipsis instead of overlapping the row buttons
- [x] Push dialog (⌘⇧K, Commit and Push, a branch's Push…), as in JetBrains: where it goes (remote, URL,
      remote branch, "New" when the branch isn't there yet), the outgoing commits and a folded file tree of
      what they change (all together or per commit), a warning when the remote has commits the branch lacks,
      Push tags; pushes `local:remote-branch` and sets the upstream (`vcs::Repo::outgoing` / `push_to`)
- [x] Branches popup: the branch submenu is its own card level with the branch row; JetBrains wording with
      the branch names ("New Branch from 'x'…", "Rebase 'main' onto 'x'", "Merge 'x' into 'main'"),
      Update and Push… (⌘⇧K) on the current branch, upstream shown in the submenu header
- [x] New Worktree… (Branches popup, Git menu, Find Action) and "New Worktree from 'x'…": location and new
      branch (empty checks out a free local branch), then opens the worktree as a project
- [x] "Show Diff with Working Tree" for any branch (one diff tab); "Tracked Branch 'origin/x'…" / "Set Tracked
      Branch…" picks the upstream from the remote branches or removes it
- [x] Push dialog: the remote is a menu when there are several, the remote branch name is editable (click it;
      the commits and files reload for the new target); Push ▾ Force Push… (`--force-with-lease`, asks first)
- [x] Word completion in every language: identifiers from the file (nearest to the caret first) and from
      other open files (most used first), listed under the language server's items without duplicates, or
      alone without a server; from 2 typed characters, prose (Markdown, text) only on ⌃Space (Basic
      Completion, also forces the list anywhere) — `search::word_completions`, `word_completion.rs`
- [x] Built-in language servers for HTML, CSS/Less, JSON, YAML, shell scripts (`.sh` only: `.env` and
      Dockerfiles share the bash highlighter), TOML, Lua, C/C++ and Swift (Apple's clangd /
      sourcekit-lsp via `xcrun`), Java, Kotlin, PHP, Elixir and Zig; LSP `languageId`s where the protocol
      names differ (`shellscript`, `typescriptreact`). The missing-server banner gets "Install in Terminal"
      (typed, not run), and a server installed meanwhile attaches when you return to the tab
- [x] Servers run through `xcrun` (clangd, sourcekit-lsp) count as installed only when `xcrun --find`
      finds the tool (cached), so the install banner shows without the Command Line Tools; `.bash`,
      `.bashrc`, `.bash_profile` and `.envrc` are shell scripts for the shell server. Markdown has no
      default server: Homebrew deprecated `marksman`
- [x] JetBrains typing for brace-less languages (`language_rules/`): Ruby, YAML, Lua, Elixir and shell
      indent after block openers (gpui-kit indentation rules, `regex`); Enter at the end of a new block's
      first line adds the body line and the closing `end` / `fi` / `done` / `esac` in one undo step (Enter is
      bound to SmartEnter, which falls back to the editor's Enter); typed `end`, `else`, `elsif`, `when`,
      `rescue`, `fi`, `done`, … line up with their open block, and get their indent back when the word grows
      (`end` → `endpoint`); Backspace in indentation goes back a level, on a blank line removes the line
      (done with the editor's own Delete/Backspace, so multiple carets keep working)
- [x] Indentation per file: `.editorconfig` (`indent_style`, `indent_size`, `tab_width`, globs, `root`),
      then the project's tab size, then the language's convention (Ruby/JS/YAML 2, Go/Make tabs, …), then
      the global tab size; also used for Reformat Code — `indentation.rs`
- [x] ⌘Q and closing a project window warn when a terminal still runs something (a running run
      configuration, or a shell whose foreground job is not the shell itself — `ps` `tpgid`, off the UI
      thread): "Terminate" / Cancel, listing the programs; Terminate replaces "Exit?" / "Close project?"
- [x] Launch reopens the projects that were open at quit (`open_projects` in `config.toml`, as tabs of one
      window); quitting from the Welcome screen brings the Welcome screen back. Welcome content is top-aligned
- [x] Branches popup: moving the pointer diagonally to a branch's submenu keeps it open (menu-aim prediction
      from the pointer's recent trail, a widened triangle to the submenu); a row crossed on the way takes
      over only once the pointer rests on it for 250 ms
- [x] `.env`, `.env.*` and `*.env` files are highlighted as shell (variable names, strings, comments); Neo
      gives `property` its own color instead of near-foreground
- [x] Ruby LSP via rbenv: when the project's Ruby lacks the server gem or isn't installed, it runs on the
      global Ruby, else the newest installed one that has it (`RBENV_VERSION`), with a fallback-mode notice and
      "Fix in Terminal" (`gem install ruby-lsp` / `rbenv install --skip-existing`)
- [x] Line edits (⌘D duplicate, ⌘/ comment, delete / move line) keep the editor's scroll position

## Backlog
Agreed backlog is done. Candidates:
Database gaps below.

## Known gaps
- Conflicts: chunks are chosen whole (no editable result pane or word-level diff like JetBrains); binary and
  delete/modify conflicts get only whole-file choices; not clicked through by hand yet
- Project tree: drag-and-drop acts on one item; right-click does not select the row; arrow keys do not
  reach the root row
- Insights: no event timeline in session details, no "tip resolved" tracking, weights/excluded
  projects/tip language not configurable in the UI, no file watcher (periodic re-index), no weekly LLM
  report (stage 4); correction detection is keyword-based (UA/EN), no similarity to the previous prompt
- Docker: logs use `docker logs -f` in a terminal tab (no in-panel search/highlight yet); no stats, no Info
  tab, no settings page (socket path, extra compose files, profiles); remote (ssh/tcp) hosts unsupported
- Debug: breakpoints are toggled with ⌘F8 (no gutter click in the gpui-kit editor), their lines do not follow
  edits, no conditional breakpoints or watches; Node.js (js-debug) not supported yet; debugpy/dlv not installed here,
  only their launch plans are unit-tested
- Replace in Files searches files as saved on disk (unsaved edits in open editors are replaced but not previewed); no regex
- `db` test `sqlite_queries_schema_and_limits` failed once; temp DB names now use a counter instead of the clock
- Navigation: Rename does not save changed open editors; ruby-lsp formats only with RuboCop installed;
  Go to Symbol lists library symbols after project ones
- "Python: current file": debugpy itself runs on `python3` from PATH, not the project venv; not clicked through by hand
- Redis TLS verified only by URL/feature tests (no TLS Redis server here)
- LightEdit: no file watching (external changes are not reloaded), unsaved text kept on quit is restored only if that folder is opened as a project; Finder opening verified only by unit tests (URL decoding), not by hand
- Dock → Quit cannot ask (gpui has no cancellable terminate hook); unsaved text is recovered instead
- PostgreSQL/MySQL/Redis drivers verified only by unit tests (no local servers); SQLite verified end to end
- The user's `lms` project needs `brew install libpq` + `bundle install` for full Ruby LSP (fallback mode works meanwhile)
