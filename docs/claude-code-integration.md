# Claude Code integration: verified facts

What the "Claude Code" settings section relies on, checked against the local CLI
(`claude --version` → **2.1.323**, `claude --help`) and the official docs on 2026-11-23.
Claude Code changes quickly: re-check this page (and bump the date) before relying on a fact.
Docs index: https://code.claude.com/docs/llms.txt

## CLI flags (cli-reference, `claude --help`)

| Flag | Use for us |
|---|---|
| `--append-system-prompt <text>` / `--append-system-prompt-file <path>` | Inject the effective IDE rules; works in interactive sessions too ("all five work in both interactive and non-interactive modes"). Appends to the default prompt. `--system-prompt[-file]` would *replace* it — never use. |
| `--settings <file-or-json>` | Session-only settings layer: above user / project / local files, below managed. Keys set here override the same keys; omitted keys keep file values; most lists merge. Writes nothing to disk. |
| `--mcp-config <configs...>` | JSON files or strings with an `{"mcpServers": {...}}` wrapper (same as `.mcp.json`). `--strict-mcp-config` would ignore other MCP servers — not used. |
| `--plugin-dir <path>` | Load a plugin directory or `.zip` for this session only; repeatable; a folder of plugins loads each child. |
| `--add-dir <dirs...>` | Extra directories with tool access (their `.claude/skills` and CLAUDE.md dirs load too). |
| `-c, --continue` | Continue the most recent conversation in the current directory. |
| `-r, --resume [id-or-name]` | Resume by session id / name, or open the picker. |
| `--model <alias-or-name>` | e.g. `opus`, `sonnet`, `fable`, or a full model name. |
| `--permission-mode <mode>` | `acceptEdits`, `auto`, `bypassPermissions`, `manual`, `dontAsk`, `plan`. |
| `--effort <level>` | `low`, `medium`, `high`, `xhigh`, `max`. |
| `-n, --name <name>` | Session display name (prompt box, `/resume`, terminal title). |
| `--session-id <uuid>` | Fixed session id (lets the IDE know which transcript is its own). |
| `--setting-sources <user,project,local>` | Limit which settings files load — not used (we never hide the user's settings). |

## Hooks (hooks.md, hooks-guide.md)

- Configured under the settings key `"hooks"`: event → matcher group → `hooks` array of handlers,
  e.g. `{"hooks":{"PostToolUse":[{"matcher":"Edit|Write","hooks":[{"type":"command","command":"…","timeout":5}]}]}}`.
  Handler types: `command`, `http`, `mcp_tool`, `prompt`, `agent`. Optional fields include `timeout` (seconds;
  default 600 for command hooks), `async`, `once`, `statusMessage`, `onFailure` (v2.1.295+).
- Events (33): SessionStart, Setup, UserPromptSubmit, UserPromptExpansion, PreToolUse, PermissionRequest,
  PermissionDenied, PostToolUse, PostToolUseFailure, PostToolBatch, Notification, MessageDisplay, SubagentStart,
  SubagentStop, TaskCreated, TaskCompleted, Stop, StopFailure, TeammateIdle, InstructionsLoaded, ConfigChange,
  CwdChanged, DirectoryAdded, FileChanged, WorktreeCreate, WorktreeRemove, PreCompact, PostCompact,
  PreModelSwitch, PostModelSwitch, Elicitation, ElicitationResult, SessionEnd.
- Stdin JSON, common fields: `session_id`, `prompt_id`, `transcript_path`, `cwd`, `scratchpad_dir`,
  `permission_mode`, `effort.level`, `hook_event_name` (+ `agent_id`, `agent_type` in subagents).
- Per event (the ones we use):
  - `PreToolUse`: `tool_name`, `tool_input`, `tool_use_id`
  - `PostToolUse`: `tool_name`, `tool_input`, `tool_response` (string for failed calls), `tool_use_id`, `duration_ms`
  - `UserPromptSubmit`: `prompt`
  - `Stop`: `stop_hook_active`, `last_assistant_message`
  - `SessionStart`: `source` (startup|resume|clear|compact|fork), `model` (may be missing)
  - `SessionEnd`: `reason`
  - `Notification`: `message`, `title`, `notification_type` (e.g. `permission_prompt`, `idle_prompt`)
  - `PreCompact` / `PostCompact`: `trigger` (manual|auto)
- Exit codes: `0` success (JSON on stdout is applied; plain stdout becomes context on some events),
  `2` blocking error, anything else non-blocking ("hook error" notice, action continues). A failing hook does
  not block unless `onFailure: "block"`; a timed-out command hook is cancelled. **Our bridge must print nothing
  and exit 0 quickly.**
- Hooks inside `--settings`: `hooks` is an ordinary settings key and `--settings` is a settings layer, but the
  docs do not show hooks passed this way explicitly → **verify with a real session before relying on it**
  (fallback: statusline + transcript).

## Status line (statusline.md)

- Settings key: `"statusLine": {"type": "command", "command": "…", "padding": 0, "refreshInterval": N}`
  (`refreshInterval` ≥ 1 s, optional).
- Stdin JSON: `session_id`, `transcript_path`, `version`, `output_style.name`, `model.id`, `model.display_name`,
  `cwd`, `workspace.current_dir`, `workspace.project_dir`, `workspace.added_dirs`, `cost.total_cost_usd`,
  `cost.total_duration_ms`, `cost.total_api_duration_ms`, `cost.total_lines_added`, `cost.total_lines_removed`,
  `context_window.total_input_tokens`, `context_window.total_output_tokens`, `context_window.context_window_size`,
  `context_window.used_percentage`, `context_window.remaining_percentage`, `context_window.current_usage`
  (may be null early), `exceeds_200k_tokens`, `effort.level`.
- Invoked on events, debounced 300 ms (in-flight runs cancelled), plus the optional timer. Its stdout is what
  Claude Code shows — a session-level `statusLine` replaces the user's, so ours must run the user's own
  command (if any) and print its output.

## Skills (skills.md)

- `~/.claude/skills/<name>/SKILL.md` (personal), `.claude/skills/<name>/SKILL.md` (project), plugin
  `<plugin>/skills/<name>/SKILL.md`; enterprise > personal > project on name clashes.
- Front-matter (YAML, all optional; `description` recommended): `name`, `description`, `when_to_use`,
  `argument-hint`, `arguments`, `disable-model-invocation`, `user-invocable`, `allowed-tools`,
  `disallowed-tools`, `model`, `effort`, `context`, `agent`, `background`, `hooks`, `paths`, `shell`,
  `metadata`, `license`, `compatibility`. Names with `<`/`>`, control or zero-width characters are refused;
  `anthropic-skills*` and `synced` are reserved. Description + when_to_use are cut at 1,536 characters.

## Plugins (plugins-reference.md)

- Optional manifest `.claude-plugin/plugin.json`; only `name` (kebab-case) is required.
- Components at the plugin root: `skills/<name>/SKILL.md`, `commands/*.md`, `agents/*.md`,
  `hooks/hooks.json` (with a top-level `"hooks"` wrapper), `.mcp.json`, `bin/`. A plugin's CLAUDE.md is **not**
  loaded. Components are namespaced: `/<plugin>:<skill>`. Check with `claude plugin validate <dir>`.

## Memory (memory.md)

- Managed (macOS) `/Library/Application Support/ClaudeCode/CLAUDE.md`; user `~/.claude/CLAUDE.md`; project
  `./CLAUDE.md` or `./.claude/CLAUDE.md`; local `./CLAUDE.local.md`; also `.claude/rules/*.md`.
- All files from the working directory up to the root load and are concatenated (root first); subdirectory
  files load on demand. `@path` imports, relative to the importing file, max depth 4.

## Settings and transcripts (settings.md, sessions.md)

- `~/.claude/settings.json` (user), `.claude/settings.json` (project), `.claude/settings.local.json` (local),
  managed settings. Precedence: managed > `--settings` > local > project > user.
- Transcripts: `~/.claude/projects/<cwd with every non-alphanumeric char → "-">/<session-id>.jsonl`
  (long names truncated + hash). The entry format is internal and may change; kept 30 days by default
  (`cleanupPeriodDays`). `CLAUDE_CONFIG_DIR` moves the whole `~/.claude` root.

## How the IDE uses this

| Feature | Mechanism |
|---|---|
| Rules (inject mode) | `--append-system-prompt-file <cache>/<project>/rules.md` |
| Skills (inject mode) | `--plugin-dir <cache>/plugin` (generated plugin `power-idea` with `skills/<name>/SKILL.md`) → skills appear as `/power-idea:<name>` |
| Hooks / gauges | `--settings <cache>/<project>/settings.json` with `hooks` (→ `power-idea claude-hook`) and `statusLine` (→ `power-idea claude-statusline`, which forwards the JSON and prints the user's own status line) |
| IDE context | `--mcp-config` with the IDE's MCP server (optional) |
| Install mode | managed block in `~/.claude/CLAUDE.md`, managed copies `~/.claude/skills/ide-<name>/` + manifest, backups |
