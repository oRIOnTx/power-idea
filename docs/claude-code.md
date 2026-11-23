# Claude Code in Power IDEA

Settings → Claude Code controls how the IDE starts [Claude Code](https://docs.claude.com/en/docs/claude-code)
and what it shows while a session runs: the IDE's **MCP server** for Claude, **live gauges** of the running
session and the **Claude terminal tab**. Rules, skills, model, permissions and hooks stay where Claude Code
keeps them (`CLAUDE.md`, `~/.claude/settings.json`, `~/.claude/skills`); the IDE does not duplicate them.
With **Enable Claude Code integration** off, the IDE starts `claude` exactly as a terminal would.

The facts this relies on (CLI flags, hook payloads, status-line JSON) are listed with their sources in
[`claude-code-integration.md`](claude-code-integration.md).

## Where things live

```
~/Library/Application Support/power-idea/claude/
  settings.toml            # the switches on this page   (schema: settings.schema.json, next to it)
~/Library/Caches/power-idea/claude/<project>-<hash>/   # generated per project: settings.json, mcp.json (0600)
```

The IDE watches `settings.toml`: editing it by hand is the same as editing in the UI. An invalid file is
reported inline on the page (with the line and field), and the last valid settings stay active.

## How the settings reach Claude Code

When the IDE starts `claude` for a project and the integration is on, it adds only:

| What | Flag |
|---|---|
| Hooks and status line for the live gauges | `--settings <cache>/settings.json` |
| The IDE's MCP server (optional) | `--mcp-config <cache>/mcp.json` (mode 0600) |

Nothing outside the IDE's folders changes, and sessions started outside the IDE are unaffected.

## Moving from the earlier Claude Code section

Earlier versions also had rules, a skills library, code quality, model / permission mode / effort and terminal
options. Claude Code does all of that natively, so they were removed:

- An old `settings.toml` still loads. The keys of removed features (`rules`, `skills`, `quality`, `terminal`,
  `general.model`, `general.permission_mode`, `general.effort`, `gauges.quality`, `gauges.in_tab_header`) are
  ignored and named in a note on the page; they disappear from the file the next time a setting changes there.
  Any other unknown key is still an error. Set the model, permission mode and effort in Claude Code itself
  (`/model`, `~/.claude/settings.json`).
- If **Install into ~/.claude** was ever used, the IDE removes, once, exactly what it added — the block between
  `<!-- ide:managed:start -->` and `<!-- ide:managed:end -->` in `~/.claude/CLAUDE.md` and the
  `~/.claude/skills/ide-*` folders marked as its own, as listed in `install-manifest.json` — and shows one
  notice listing it. Your own text and folders are never touched; old backups stay in `backups/`.
- The `rules/` and `skills/` folders in the IDE's settings folder are left as they are; delete them when you no
  longer need them.
- A rules block written into a project's `CLAUDE.md` with **Export to project** is not removed automatically;
  delete the lines between the two `ide:managed` markers by hand.

## Live gauges

Fed by Claude Code itself while a session started by the IDE runs: hook events (activity, files changed),
the status line JSON (context window %, tokens, cost) and, when no status line arrives, the
session transcript (parsed by the same code as Insights). Gauges: context window ring (green → amber → red at
the thresholds you set), tokens and cost, current activity, files changed. Each can be
switched off, and each place (status bar, sidebar); the Claude tab header shows only the tab name. Values are always shown as text too.

The hooks run `power-idea claude-hook`, which forwards the JSON to the IDE over a local Unix socket and exits
0 immediately (nothing printed, nothing blocked — if the IDE is closed it is a no-op). The status line runs
`power-idea claude-statusline`, which forwards its JSON and prints **your own** status line command (from
your Claude settings) — or a short `Model · ctx 42% · $0.37` when you have none.

## Claude folder

Shortcuts to Claude Code's own files, so there is no need to duplicate them in the IDE:

- **Edit Claude settings** opens `~/.claude/settings.json` (model, permissions, hooks, …) in an editor tab.
- **Edit global rules** opens `~/.claude/CLAUDE.md` — the instructions Claude Code loads in every project — with
  the Markdown preview.
- **Open ~/.claude** shows the folder in Finder.

A missing file is created only when you click its button: `{}` for `settings.json`, empty for `CLAUDE.md`. The
IDE never writes its own content into these files. `CLAUDE_CONFIG_DIR` is honored like Claude Code does.

## The Claude tab

The **Claude** icon in the activity bar (and **⌘Esc**, as in Claude Code's JetBrains plugin; remap it in
`keymap.toml` as `ide::ToggleClaude`) shows and focuses the project's Claude tab, or opens a new one running
`claude` in the project root right away. Pressed again while that tab has focus, it returns to the editor. The
icon is always there: with the integration off, `claude` starts exactly as a terminal would; with it on, it
gets the IDE's additions above. A green dot on the icon means a Claude tab's process is running (from the
terminal's process state only).

Right-click the icon for **New Session**, **Continue Last Session** (`--continue`), **Resume Session…**
(`--resume`, Claude Code's own picker) — each opens a new Claude tab — and **Restart** / **Close** for the
current Claude tab (the active one, else the last). All of them are in Find Action too. Closing a running session
asks first; a missing `claude` shows how to install it. The Run widget's Stop and ⌘F2 never stop a Claude tab,
and ⌘T / **+** in the terminal panel always open a plain shell.

## Privacy

The IDE reads only local files — its own folder, your Claude settings and the transcripts in
`~/.claude/projects` — and sends nothing anywhere. Claude Code itself talks to Anthropic as usual.
