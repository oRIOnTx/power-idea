# Claude Code in Power IDEA

Settings → Claude Code configures how the IDE works with [Claude Code](https://docs.claude.com/en/docs/claude-code):
global **rules** mixed into the context of matching projects, a **skills** library, **code quality**
tracking, **live gauges** of the running session, and a pinned **Claude terminal tab**. Every feature has its
own switch, and the whole section is off by default — with it off the IDE behaves exactly as without it.

The facts this relies on (CLI flags, hook payloads, status-line JSON, skill and plugin layout) are listed with
their sources in [`claude-code-integration.md`](claude-code-integration.md).

## Where things live

```
~/Library/Application Support/power-idea/claude/
  settings.toml            # all switches and options   (schema: settings.schema.json, next to it)
  rules/*.md               # one rule per file           (schema: rules/rule.schema.json)
  skills/<name>/SKILL.md   # standard Claude Code skills (built-ins are copied here once, then yours)
  install-manifest.json    # what "Install into ~/.claude" added (only while installed)
  backups/                 # copies of files before the IDE first changed them
~/Library/Caches/power-idea/claude/<project>-<hash>/   # generated per project; "Clear IDE caches" removes it
```

The IDE watches this folder: editing a file by hand — or letting an AI agent edit it — is the same as editing
in the UI. Invalid files are reported inline in Settings → Claude Code (with the line and field), and the last
valid settings stay active.

## How the settings reach Claude Code

Each feature (rules, skills) has a **mode**:

- **Inject** (default): when the IDE starts `claude` (the Claude tab), it builds the effective configuration
  for the open project and passes it with CLI flags pointing at generated files in the cache folder:

  | What | Flag |
  |---|---|
  | Rules that apply to the project, highest priority first | `--append-system-prompt-file <cache>/rules.md` |
  | Enabled skills, as the plugin `power-idea` (`/power-idea:<skill>`) | `--plugin-dir <cache>/plugin/power-idea` |
  | Hooks and status line for the gauges | `--settings <cache>/settings.json` |
  | The IDE's MCP server (optional) | `--mcp-config <cache>/mcp.json` (mode 0600) |
  | Model, permission mode, effort | `--model`, `--permission-mode`, `--effort` |

  Nothing outside the IDE's folders changes, and sessions started outside the IDE are unaffected.

- **Install** (opt-in): writes a managed block into `~/.claude/CLAUDE.md` (between
  `<!-- ide:managed:start -->` and `<!-- ide:managed:end -->`) and managed copies of skills as
  `~/.claude/skills/ide-<name>/`, so they apply to `claude` run anywhere. Files are backed up before their
  first change; **Uninstall** removes exactly what was added (a file the IDE created is deleted again, your
  own text and skills are never touched). Scoped rules are installed with a line stating their scope, since
  the IDE cannot match projects outside its own sessions.

**Precedence:** the project's own `CLAUDE.md` files and `.claude` settings win over IDE rules. The injected
text says so explicitly, and **Preview effective context** shows exactly what would be sent for the open
project — the command line, the environment and every generated file (tokens redacted).

**Export to project** is the only way the IDE writes into a repository: it puts the rules that apply to the
project into a managed block of the project's `CLAUDE.md`, idempotently, when you click it.

## Writing rules

A rule is a Markdown file with YAML front-matter:

```markdown
---
name: "Rust style"
enabled: true
description: "How we write Rust"
applies_to:
  languages: ["rust"]          # or paths: ["src/**/*.rs"], projects: ["~/work/*"], or applies_to: "all"
priority: 50                    # higher comes first
tags: ["style"]
---

- Prefer `let … else` over nested `match` for early returns.
- No `unwrap()` on I/O or user input.
```

- `applies_to: "all"`, or any combination of `languages` (detected from file extensions: `rust`,
  `typescript`, `javascript`, `python`, `ruby`, `go`, `java`, `kotlin`, `swift`, `c`, `cpp`, `csharp`, `php`,
  `elixir`, `scala`, `shell`, `sql`, `vue`, `svelte`, `lua`, `zig`, `dart`, `docker`), `paths` (globs over
  project-relative paths) and `projects` (roots or globs, `~/` expanded). A project matches when **any**
  listed criterion does.
- Unknown keys are errors, so typos do not pass silently. The schema is in `rules/rule.schema.json`.
- In the UI: drag rows (or use ↑/↓) to reorder — priorities are renumbered 10, 20, 30…; **Test** checks a
  rule against the open project; **Edit** opens the file in an editor tab with Markdown preview (Find Action → Toggle Preview);
  **New from template** offers coding style, testing policy, commit format, security and architecture notes.

### With an AI agent

Point the agent at the folder and the schemas, e.g. *"Add a rule in ~/Library/Application Support/power-idea/claude/rules
for our Python projects: type hints everywhere, pytest for tests. Follow rules/rule.schema.json."* The IDE
picks the file up immediately; check it in Settings → Claude Code (errors are shown there) and with
**Preview effective context**.

## Skills

Standard Claude Code skills: a folder with a `SKILL.md` (YAML front-matter with at least a `description`,
then instructions; scripts and references may sit next to it). The IDE ships four editable built-ins —
`code-review`, `write-tests-first`, `refactor-safely`, `conventional-commits` — copied into the folder once
(delete one and it stays deleted). Each skill has a switch; **Add from folder…** and **Add from Git URL…**
(https or git@, shallow clone) copy every `SKILL.md` folder they find. Skills that the project's Claude
session started with are marked *active in session*.

## Code quality

When switched on, the IDE detects the project's linters (`cargo clippy` + `cargo fmt --check`, eslint,
prettier, rubocop, ruff, `go vet`), runs them after Claude edits a file (PostToolUse hook), on save or only
from **Run now**, and scores the result:

> score = 100 − error weight × errors − warning weight × warnings, clamped to 0…100 (defaults 5 and 1)

The quality ring shows the score and the change in errors / warnings since the session started.

## Live gauges

Fed by Claude Code itself while a session started by the IDE runs: hook events (activity, files changed,
quality runs), the status line JSON (context window %, tokens, cost) and, when no status line arrives, the
session transcript (parsed by the same code as Insights). Gauges: context window ring (green → amber → red at
the thresholds you set), tokens and cost, current activity, files changed, code quality ring. Each can be
switched off, and each place (status bar, sidebar); the Claude tab header shows only the tab name. Values are always shown as text too.

The hooks run `power-idea claude-hook`, which forwards the JSON to the IDE over a local Unix socket and exits
0 immediately (nothing printed, nothing blocked — if the IDE is closed it is a no-op). The status line runs
`power-idea claude-statusline`, which forwards its JSON and prints **your own** status line command (from
your Claude settings) — or a short `Model · ctx 42% · $0.37` when you have none.

## The Claude tab

With **Terminal → Claude tab** on, opening the terminal panel (⌥F12) starts a pinned first tab running
`claude` in the project root with the injected configuration — one per project window. Options: start
automatically or on Enter, `--continue` the last session, restart from the tab header. A missing `claude`
shows how to install it; closing a running session asks first.

## Privacy

The IDE reads only local files — its own folder, your Claude settings and the transcripts in
`~/.claude/projects` — and sends nothing anywhere. Claude Code itself talks to Anthropic as usual.

## Demo

1. Settings (⌘,) → **Claude Code** → switch on **Enable Claude Code integration**; the General card shows the
   detected `claude` and its version.
2. Rules → **New from template → Coding style**; in the editor that opens change `applies_to` to
   `languages: ["rust"]` and save. The list shows the **Rust** chip.
3. Open a Rust project (e.g. this repository). In the rule's row click **Test** → "applies to …".
4. Click **Preview effective context**: the command line contains `--append-system-prompt-file` and the
   `rules.md` section shows your rule under "Power IDEA rules".
5. Make sure **Terminal → Claude tab** and **Live status** are on, then press ⌥F12: the **Claude** tab starts
   first. Ask Claude something that edits a file: the status bar shows the context ring,
   tokens / cost, the activity ("Editing src/…") and the files-changed count; with **Code quality** on, the
   quality ring appears after the first edit.
