# Power IDEA — progress

Updated: 2026-11-25. Last commit: see `git log`. Everything below marked *uncommitted*
is in the working tree only.

## Done

### Core (committed)
- [x] Cargo workspace: `search`, `vcs`, `mcp`, `app` crates; project rules (`CLAUDE.md`) and lints
- [x] Editor with tree-sitter highlighting (~35 languages), tabs, JetBrains line operations
- [x] Project tree, Search Everywhere (⇧⇧), Go to File (⌘⇧O), Recent Files (⌘E), Find in Files (⌘⇧F), Find Action (⌘⇧A), Go to Line (⌘L)
- [x] Integrated terminal (⌥F12, tabs, copy/paste, selection, scrollback)
- [x] Terminal tabs: Rename… (context menu or double-click; Enter saves, Esc cancels, empty restores the
      default), Pin / Unpin (pin icon, first, no ×; ⌘W refuses), Close Tab, Close Other Tabs (keeps pinned).
      Pinned shells are saved per project in `<config>/workspaces/<key>/pinned_terminals.json` (name, order,
      directory relative to the root; debounced atomic writes) and reopen once, in their last directory, the
      first time the panel opens. Directory comes from the shell process (`lsof`, `/proc` on Linux) every 3 s;
      OSC 7 is not used (alacritty_terminal 0.26 does not surface it)
- [x] Terminal shells start directly as login shells (no `login(1)` "Last login" banner) with
      `TERM_PROGRAM=power-idea`, so Terminal.app's inherited session restore no longer runs in IDE tabs
- [x] Bottom tool windows span the full window width under the Project tree (Settings → Terminal → Full window width; on by default)
- [x] `.power-idea` folder hidden in the Project tree; Settings → Appearance → Project tree → Show .power-idea folder
- [x] Settings → System page: "Open projects in" and Confirmations (exit / close project) moved out of Appearance
- [x] **LightEdit / quick editor**: files opened from Finder (Open With, Dock icon), `power-idea <files…>` or File → Open…
      (now files or folders) open in the project that contains them, else in a LightEdit window (no index, git,
      language servers or saved state, nothing written next to the file). The bundle registers ~190 text
      extensions (`crates/app/text-extensions.txt`) plus folders; more extensions map to highlighters (ini/cfg → TOML, xml/plist → HTML, …)
- [x] Git: status colors, Commit tool window (⌘0), side-by-side diffs, push/pull/fetch, branches, rollback
- [x] MCP server for Claude Code (17 tools, +3 read-only CI tools later, token in Keychain-like file, localhost only)
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
- [x] Self-contained scripts: `scripts/run.sh`, `check.sh`, `build.sh` (find rustup themselves)
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
- [x] **Discover Databases…** (data source menu, PostgreSQL / MySQL): lists the databases the user may use
      over the open connection (PG: `has_database_privilege(…, 'CONNECT')`, MySQL: `SHOW DATABASES`), searchable
      multi-select dialog, shown ones preselected (missing ones kept and marked). Picked databases become tree
      nodes (`databases` in `databases.toml`) that connect lazily, one pool each (the data source's own database
      reuses its connection); their menu: Open Console, Refresh. Not clicked through by hand yet.
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
- [x] Editor: Tab accepts the highlighted completion item (JetBrains), otherwise indents (`SmartTab`)
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
- [x] **Extension API v1** (`crates/extensions`, `docs/extensions.md`): an extension is any program in
      `<config>/extensions/<id>/` with an `extension.toml`, run per project window and talking JSON-RPC 2.0
      over stdio (LSP framing). Commands go to Find Action ("Name: Title") with optional key bindings;
      requests: workspace info, open file, active editor, text, undoable edits, selection, notifications,
      pickers (`ui/pick` via the palette), input dialogs, terminal runs; events: file opened / closed / saved /
      changed, active editor changed. Lazy start (eager with events), crash → notification + restart on next
      command, stderr → `~/Library/Logs/Power IDEA/extensions/<id>.log`; Reload Extensions / Open Extensions
      Folder. Python example in `examples/extensions/hello-world`, run end to end by `cargo test -p extensions`
- [x] Dialog buttons 1.25× larger: confirmations (`style::confirm_buttons`, 32 → 40 px, same Confirm/Cancel
      actions so Enter/Esc/`on_ok` keep working) and hand-built dialog footers (`style::dialog_button`, 24 → 30 px);
      also the Push dialog (split button sized in pixels so both halves stretch), About (`style::ok_button`) and
      form buttons inside dialogs (database kind / Choose…, New Project Browse…)
- [x] Git Log commit details (changed files + message) scroll: rows used to shrink to fit instead of overflowing
- [x] **Settings → Claude Code** (`crates/claude_integration`, `crates/app/src/claude/`, docs `docs/claude-code.md`,
      verified CLI facts `docs/claude-code-integration.md`): off by default; `claude/settings.toml` (JSON Schema
      kept in sync by a test, inline errors, hot-reloaded). When on, the IDE's `claude` gets `--settings` (hooks +
      status line, verified with a real session) and optional `--mcp-config` (0600). Hook bridge
      `power-idea claude-hook` / `claude-statusline` over a Unix socket (exit 0, user's status line kept) →
      gauges (context ring with thresholds, tokens/cost with transcript fallback via the Insights parser,
      activity, files changed) in the status bar and sidebar. Claude terminal tabs: restart from the header,
      install hint, close confirmation; the Run widget's Stop and ⌘F2 ignore them.
- [x] **Claude Code section slimmed down** (2026-11-23): removed what Claude Code does natively — rules (Inject /
      Install, templates, import/export, preview, export to project), skills library and plugin, code quality
      and its ring, model / permission mode / effort, the Terminal group (⌘T / + now always open a shell) and
      Privacy / Clear caches. Old `settings.toml` files load: `REMOVED_KEYS` are ignored and named on the page.
      `claude_integration::legacy::cleanup` removes, once, what Install mode put into `~/.claude` (manifest-driven,
      marker-checked, tested); `rules/` and `skills/` in the IDE folder are kept.
- [x] **⌘Esc → Claude tab** (`ide::ToggleClaude`, remappable): focus the project's Claude tab or start one;
      again from that tab → back to the editor. Find Action: New Session, Continue (`--continue`), Resume…
      (`--resume`), Restart, Close (asks when running). `launcher::Start` picks the flag. The activity bar icon
      was removed (2026-11-25).
- [x] **Live status gauges removed** (2026-11-23): no more hooks / status line injected via `--settings`, the
      `claude-hook` / `claude-statusline` bridge, the Unix socket and the gauges in the status bar and sidebar.
      `gauges` joined `REMOVED_KEYS`; the two helper subcommands still exit quietly for sessions started earlier.
- [x] **Claude folder** section in Settings → Claude Code: Edit Claude settings (`~/.claude/settings.json`),
      Edit global rules (`~/.claude/CLAUDE.md`, Markdown preview), Open ~/.claude; a missing file is created only
      on click (`{}` / empty, `claude_integration::home::ensure_file`, never overwrites).
- [x] **Versioning from git** (spec `docs/backlog/versioning-and-changelogs.md`): `scripts/version.sh` (annotated
      `v*` tag → version, commit count → build, `X.Y.Z (N)` on a tag / `X.Y.Z-dev.N+sha[.dirty]` after it,
      `APP_VERSION`/`APP_BUILD` without git) feeds `build.rs`, the bundle's Info.plist and the Welcome / About
      version (click or "Copy Version Details" copies version + build + commit); `bump-version.sh` removed.
      Conventional Commits with a `cog verify` commit-msg hook (lefthook, `bin/setup`); `CHANGELOG.md` from
      git-cliff (`cliff.toml`, Keep a Changelog; pre-convention "Add/Fix/Remove …" commits sorted by verb, doc-only
      ones skipped) via `bin/changelog`; `bin/release` (dirty/branch guard, auto bump, plan + confirm, changelog
      section, `release-notes/vX.md`, `chore(release)` commit, annotated tag, no push). No tag exists yet
- [x] **Quality / security / CI pipeline** (spec `docs/backlog/quality-pipeline.md`): `rust-toolchain.toml` (1.100.0),
      MSRV 1.98.1 (`cargo msrv`); strict `[workspace.lints]` (clippy pedantic + unwrap/expect/panic/indexing/
      print_stdout, noisy pedantic allowed with reasons; tests exempt via `clippy.toml`) — all findings fixed;
      taplo, typos, editorconfig, cargo-deny (gpui-only unmaintained crates ignored with reasons), `.cargo/audit.toml`;
      lefthook hooks (pre-commit fmt/taplo/typos/gitleaks, commit-msg `cog verify`, pre-push clippy/nextest/deny);
      `justfile` (`just check` = CI); GitHub Actions: `ci.yml` (fmt, lint, typos, machete, test, msrv, deny, coverage
      ≥ 34 % lines, gitleaks, actionlint + zizmor), `audit.yml` (daily, opens an issue), `codeql.yml`,
      `scorecard.yml`, `release.yml` (tag → dmg + SHA256SUMS + CycloneDX SBOM + provenance attestation + GitHub
      Release from `release-notes/`), dependabot (weekly, grouped, 7-day cooldown); actions pinned to SHAs,
      `permissions: contents: read`, `persist-credentials: false`. CODEOWNERS, PR/issue templates, CONTRIBUTING,
      SECURITY. CodeQL is skipped while the repository is private; Scorecard is optional (manual run, `SCORECARD_TOKEN`
      secret for a private repo);
      cargo-machete only in CI (local install was not allowed)
- [x] **Error journal** (`crates/error_journal`, `app/src/errors/`): panics on any thread (synchronous write
      before a possible abort, `try_lock` + `panic-<ts>.json` fallback merged on the next start), `log::error!`
      (gpui's errors) and `error_journal::report(&anyhow, context)`; fingerprint dedup (×count), trimmed
      backtraces (own frames ± 2, ≤ 60 lines / 8 KB), home → `~`, tokens redacted; `errors.json` in the config
      dir, ≤ 50 records / 256 KB, atomic writes from a debounced worker, hourly pruning; reset on a new
      version/build; crash detection via `clean_shutdown`. Retention ("Until restart" keeps an unseen crash) in
      Settings → System and the Errors tab. Red "!" on the settings gear (tooltip + accessible label "N new
      errors"), click → Errors tab (newest first, expand → context/location/version/backtrace, Copy as Markdown,
      Open file location, Copy all, Clear all); gear right-click menu, Help → Show Errors, Find Action. Debug
      builds: "Debug: Trigger Test Error / Panic / Crash" in Find Action
- [x] **GitHub Actions tab, phase 1** (2026-11-25; `crates/ci`, `app/src/github_actions/`, ⌘8, activity bar,
      View menu, Find Action): read-only runs of the project's GitHub repository (`origin`, else the first GitHub
      remote; ssh/https/scp URLs). Token from `gh auth token`, else a personal access token saved in the Keychain
      (`security -i` over stdin, never in argv); "Connect to GitHub" explains both (gh login typed into a terminal,
      PAT field + link). REST via `ureq` (rustls; approved dependency): ETag/304 cache, `x-ratelimit-*` tracking,
      friendly 401/403/404/rate-limit errors, job logs fetched through the 302 without forwarding the token.
      Adaptive polling (`ci::poller`): 4 s while something runs and the tab is in front, 30 s idle, 60 s for the
      HEAD badge when the tab is hidden, paused when the window is inactive; exponential backoff, waits out the
      rate-limit reset. Layout: header (repo, branch, HEAD passing/failing/running chip, This branch / All, log
      search, "Updated Xs ago", refresh, Open on GitHub), resizable runs list | run detail. HEAD's failed run opens
      by itself with the failed job and step unfolded and the log scrolled to the first error. Detail is one
      virtualized list: jobs → annotations (click opens file:line) → steps (spinner + ticking duration, copy step
      log) → log lines (timestamps/ANSI stripped, `##[group]` depth, errors/warnings/commands colored, line
      numbers per step; lines matched to steps by time). Keys: ↑/↓, Enter (open / fold), Esc (back, clear search),
      O (GitHub, at the step), ⌘R, ⌘F/⌃F (Enter / ⇧Enter next/previous match), click / ⇧click + ⌘C copies lines.
      Activity bar dot: red when HEAD fails, amber while it runs
- [x] **GitHub Actions tab, phase 2** (2026-11-25): Run workflow… (D, header and run detail): workflows with
      `workflow_dispatch` (parsed from the checked-out YAML via tree-sitter-yaml, else the default branch's file through
      the contents API; `ci::yaml`, `ci::dispatch`), others disabled with the reason; ref (current branch by default)
      and inputs (string / number / environment fields, boolean checkbox, choice buttons; required and type checks).
      After the dispatch the new run is found (`event=workflow_dispatch`, created since the dispatch) and opened.
      Run actions in the detail header: Re-run failed jobs (R), Re-run all, Cancel (each one click, contextual).
      Deploy workflows (name / file contains `deploy`, `release`, `publish`, `prod`, or `[ci] deploy_patterns` in
      `.power-idea/settings.toml`) ask before a dispatch or re-run. Runs on HEAD and runs started from the IDE are
      watched: when one finishes, an in-app notification (red on failure), plus a macOS notification via `osascript`
      while the IDE is in the background (polling continues at 60 s for them). CI chip (✓ ✗ ●) next to the branch
      in the Commit panel opens the tab
- [x] **GitHub Actions tab, phase 3** (2026-11-25): Send to Claude (C; run header, bot icon on a failed step)
      builds a Markdown block (`ci::context`): workflow / job / step, repo, branch, commit, run URL, annotations, the
      failed step's log around its error lines (±20, merged, ≤ 150 lines, omitted stretches marked; the last lines when
      nothing looks like an error) and the task; trimmed to `[ci] claude_context_chars` (default 12 000) and redacted
      (`ci::redact`: GitHub/AWS/Slack/Stripe/OpenAI-style keys, JWTs, PEM keys, `Authorization:` and
      `password=`/`token:`-like values, URL credentials). It is pasted (bracketed, not sent) into the project's Claude
      tab once Claude's prompt is ready (a new tab is started when needed; clipboard fallback after 20 s); Copy puts it
      on the clipboard. The job log is fetched if it was not loaded. MCP tools (read-only, `ci::tools`):
      `ci_list_runs(branch?, limit?)`, `ci_get_run(run_id)` (jobs, steps, failed jobs' annotations),
      `ci_get_failed_logs(run_id)` (the same blocks per failed job); they work without opening the tab (gh / Keychain
      token). No tool starts, re-runs or cancels runs
## Backlog
- GitHub Actions: `ci_rerun_failed` MCP tool behind a setting (off by default), if ever wanted

Other candidates: Database gaps below.

## Known gaps
- GitHub Actions: UI not clicked through by hand; live API checked only by ignored tests
  (`cargo test -p ci -- --ignored`); logs exist only after a job finishes (no live streaming); log search covers
  unfolded steps only; ANSI colors are stripped, not rendered; github.com only (no Enterprise hosts); the HEAD
  badge and Commit-panel chip appear once the tab has been opened in the window (lazy, like Docker); dispatch
  inputs come from the local / default-branch YAML even when another ref is chosen; macOS notifications come from
  `osascript` (shown as Script Editor, clicking does not open the IDE)
- Error journal: UI not clicked through by hand; native crashes (SIGSEGV, aborts in foreign code) are not
  captured (TODO in the crate docs); release builds are stripped, so their backtraces have no symbols and the
  fingerprint falls back to the message
- Claude Code section: UI not clicked through by hand (logic is unit-tested, hooks via --settings verified once
  with a real `claude -p`); the gauge sidebar block sits under every sidebar panel; managed blocks written by the
  old Export to project stay in project `CLAUDE.md` files until removed by hand
- Extensions: UI not clicked through by hand yet (protocol and the example are tested end to end); no
  settings page listing extensions, no status bar items, panels or custom editor decorations; `keymap.toml`
  cannot rebind extension commands; events reach only already running processes; not loaded from project
  folders on purpose
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
