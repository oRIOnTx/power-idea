<div align="center">

<img src="assets/icon.svg" width="112" alt="Power IDEA icon">

# Power IDEA

**A fast, language-agnostic desktop IDE in Rust with JetBrains ergonomics.**

GPU-rendered UI, the JetBrains keymap you already know, language servers and
debuggers for any language, Git, databases, a real terminal — and an MCP server
so Claude Code can see what you see.

![Rust](https://img.shields.io/badge/Rust-2024-orange?logo=rust)
![macOS](https://img.shields.io/badge/macOS-13%2B-black?logo=apple)
![License](https://img.shields.io/badge/license-MIT-blue)

</div>

---

## Screenshots

| Editor, project tree and Find Usages | Git Log with commit graph |
|---|---|
| ![Editor](docs/screenshots/editor.png) | ![Git Log](docs/screenshots/git-log.png) |
| **Debugging (rdbg, lldb-dap, debugpy, dlv)** | **Database consoles** |
| ![Debug](docs/screenshots/debug.png) | ![Database](docs/screenshots/database.png) |

<p align="center"><img src="docs/screenshots/welcome.png" width="70%" alt="Welcome screen"></p>

## Features

### Editor & code intelligence
- Tree-sitter highlighting for ~35 languages, tabs, JetBrains line operations (⌘D, ⌘⌫, ⌥⇧↑/↓, ⌘/).
- **Language Server Protocol** for any language: completion, hover, diagnostics,
  go to definition (⌘B / ⌘+click), back/forward (⌘[ / ⌘]).
- **Navigation**: Go to Class (⌘O), Go to Symbol (⌘⌥O), File Structure (⌘F12),
  Find Usages tool window (⌥F7), Rename (⇧F6), Reformat Code (⌥⌘L).
- Live **previews**: Markdown and SVG next to the editor (editor / split / preview), image viewer.

### Search
- Search Everywhere (⇧⇧), Go to File (⌘⇧O), Recent Files (⌘E), Find Action (⌘⇧A).
- Find in Files (⌘⇧F) and **Replace in Files** (⌘⇧R) with a live preview of every change.

### Run & Debug — any language
- **Run configurations** detected from the project (Rails, RSpec, Cargo, npm/yarn/pnpm/bun,
  Go, pytest, Django, Mix, Make) or written in `.power-idea/run.toml`. Each runs in its own
  terminal tab — colors, input, `pry`/`byebug` just work. Run ⌃R, Stop ⌘F2.
- **Debugging through the Debug Adapter Protocol**: rdbg (Ruby), lldb-dap (Rust, C/C++),
  debugpy (Python), dlv (Go). Breakpoints (⌘F8), Debug ⌃D, step over/into/out (F8/F7/⇧F8),
  frames, variables, console with expression evaluation.
- ⌘+click on `file:line` in any terminal output (stack traces, compiler errors) opens it.

### Git
- Commit tool window (⌘0) with changelists-style selection, amend, commit & push.
- **Git Log** (⌘9): commit graph, branches panel with ahead/behind, filters
  (`author:`, `path:`, `after:`, `before:`), cherry-pick, revert, reset, merge, rebase, conflict banner.
- Side-by-side diffs, push / pull / fetch, branch switching.

### Database
- PostgreSQL, MySQL/MariaDB, SQLite and Redis (with TLS); passwords in the macOS Keychain.
- Schema tree, SQL consoles (⌘↩) with table/column/alias completion.
- Edit cells, add and delete rows — submitted in one transaction. Export to CSV.

### Terminal
- Real terminal (alacritty_terminal) with tabs, scrollback, mouse support for TUI apps
  (vim, htop, lazygit), blinking cursor.

### Projects & workspace
- Welcome screen with recent projects and New Project (optional `git init`).
- Several projects as tabs of one window; open tabs, cursors and panels restored per project.
- Follows changes made outside the IDE (terminal, `git pull`, AI agents).
- Unsaved edits survive a Dock → Quit and come back on the next start.

### AI: MCP server for Claude Code
A local, token-protected MCP server exposes read-only tools: open files and the active
editor, project search, git status/diff/log, **LSP diagnostics, symbols, usages, file structure**,
plus "open this file / show this diff" navigation. Agents can look, but never write files or run commands.

## Build

Requirements: macOS 13+, Xcode Command Line Tools, and Rust via rustup.

```sh
xcode-select --install          # compilers, lldb-dap
brew install rustup && rustup-init
```

All scripts find the Rust toolchain on their own:

```sh
scripts/run.sh [project-dir]     # debug build, opens a project
scripts/build.sh                 # release "Power IDEA.app" in target/release/, then opens it
scripts/check.sh [--live]        # fmt + clippy + tests (--live: also against real ruby-lsp, rdbg, lldb-dap)
scripts/bump-version.sh minor    # semantic version; the build number comes from git automatically
```

macOS remembers folder permissions per signing identity. Create a local signing
certificate once so they survive rebuilds:

```sh
scripts/create-signing-cert.sh
```

## Language servers and debuggers

Power IDEA uses whatever is installed; missing tools are offered in Settings → Languages.

| Language | Language server | Debugger |
|---|---|---|
| Ruby | `gem install ruby-lsp` | `gem install debug` (rdbg) |
| Rust | `rustup component add rust-analyzer` | lldb-dap (Xcode CLT) |
| TypeScript / JavaScript | `npm i -g typescript-language-server typescript` | — |
| Python | `npm i -g pyright` | `pip install debugpy` |
| Go | `go install golang.org/x/tools/gopls@latest` | `go install github.com/go-delve/delve/cmd/dlv@latest` |

## Connect Claude Code

Click **MCP** in the status bar — it copies the command with your local token:

```sh
claude mcp add --transport http power-idea http://127.0.0.1:<port>/mcp --header "Authorization: Bearer <token>"
```

## Configuration

| File | What |
|---|---|
| `~/Library/Application Support/power-idea/config.toml` | appearance, editor, terminal, language servers |
| `~/Library/Application Support/power-idea/keymap.toml` | your key bindings on top of the JetBrains defaults |
| `<project>/.power-idea/settings.toml` | project settings (excludes, tab size) |
| `<project>/.power-idea/run.toml` | run and debug configurations |
| `<project>/.power-idea/databases.toml` | data sources (no passwords) |
| `<project>/.power-idea/workspace.toml` | personal state (open tabs…), git-ignored |

Example `run.toml`:

```toml
[[config]]
name = "RSpec"
command = "bundle exec rspec"
env = { RAILS_ENV = "test" }

[config.debug]          # Debug ⌃D
adapter = "ruby"        # ruby | lldb | python | go
```

## Architecture

```
crates/
  app/     the gpui application (the only crate that depends on gpui-kit)
  search/  file index, fuzzy matching, find & replace in files
  vcs/     git through the system git CLI
  lsp/     Language Server Protocol client
  dap/     Debug Adapter Protocol client and adapter launch plans
  run/     run configurations and file:line links in program output
  db/      PostgreSQL / MySQL / SQLite (sqlx) and Redis
  mcp/     MCP server (JSON-RPC over local HTTP)
```

Logic lives in UI-free crates with unit tests; the `app` crate wires state to views.
See [`CLAUDE.md`](CLAUDE.md) for the conventions and [`PROGRESS.md`](PROGRESS.md) for what is done and what is next.

## License

MIT
