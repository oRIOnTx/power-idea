# Power IDEA extension API (v1)

An extension is any program — Python, Node, Ruby, Rust, a shell script — that Power IDEA starts as a
child process and talks to over stdin/stdout with **JSON-RPC 2.0**, framed like the Language Server
Protocol (`Content-Length: N\r\n\r\n{json}`). If you have written an LSP server, you already know the
transport. A crashing extension cannot take the IDE down.

A complete example covering every method: [`examples/extensions/hello-world`](../examples/extensions/hello-world)
(Python, standard library only; its `Connection` class is a reusable ~100-line client).

## Installing

```
~/Library/Application Support/power-idea/extensions/
  hello-world/
    extension.toml
    main.py
```

Each folder with an `extension.toml` is one extension. After adding or changing one, run
**Reload Extensions** (Find Action, ⌘⇧A); **Open Extensions Folder** shows the folder in Finder.
Load errors (bad manifest, duplicate ids, bad key bindings) are shown as a notification.

Extensions run with your user's permissions, like VS Code or JetBrains plugins: install only ones you
trust. They are never loaded from a project's folder, so opening a repository cannot run its code.

## `extension.toml`

```toml
id = "hello-world"          # required; lowercase letters, digits, - _ .
name = "Hello World"        # shown in Find Action ("Hello World: Greet"); defaults to id
version = "0.1.0"
description = "…"
command = "python3"         # required: a program on PATH, or a path relative to this folder ("./bin/ext")
args = ["main.py"]          # the process runs with this folder as its working directory
events = ["file_saved"]     # optional, see Events

[[commands]]
id = "hello-world.greet"    # unique across all extensions; prefixing with your id is recommended
title = "Greet"
keybinding = "cmd-alt-shift-h"   # optional, gpui syntax; active in project windows
```

Unknown keys are errors, so typos do not go unnoticed. Key bindings come only from `keybinding`;
`keymap.toml` cannot target extension commands yet. A user binding for the same keys wins.

## Lifecycle

- One process **per extension per project window**. Its environment has `POWER_IDEA_PROJECT` (project
  root) and `POWER_IDEA_API_VERSION`.
- An extension with `events` starts when the project opens; one without starts on its first command.
- LightEdit windows (single files opened from Finder) do not run extensions.
- The IDE sends `initialize` first and waits up to 10 s for the answer.
- When the window closes (or on Reload Extensions) the IDE sends `shutdown` and kills the process
  0.5 s later.
- If the process exits, the IDE shows "Extension … stopped" with the last stderr line; the next
  command starts it again. A failing start is reported the same way.
- **stdout is the protocol.** Log to stderr: it is appended to
  `~/Library/Logs/Power IDEA/extensions/<id>.log`.

## IDE → extension

| Message | Kind | Params | Expected result |
|---|---|---|---|
| `initialize` | request | `{apiVersion, ide: {name, version}, project: {root, name}, extension: {id, dir}}` | anything (`{}`) |
| `command/execute` | request | `{command}` — a command id from the manifest | anything; an error is shown to the user |
| `event` | notification | `{type, path}` | — |
| `shutdown` | notification | none | exit |

Commands may take as long as they need (up to 10 minutes) — e.g. while waiting on `ui/pick`. Handle
`command/execute` off your reader loop so you can keep reading answers to your own requests (the
example uses a thread per command).

## Extension → IDE

Requests are answered with `result`, or with a JSON-RPC `error` (`-32601` unknown method, `-32602` bad
params, `-32603` failed, e.g. a path outside the project). Params may be omitted when all are optional.

**Positions** are `{line, column}`, both **1-based**; columns count Unicode characters. A range is
`{start, end}`. **Paths** are relative to the project root (absolute paths inside it are accepted);
anything outside the project is rejected.

| Method | Params | Result |
|---|---|---|
| `workspace/info` | — | `{root, name, branch, openFiles: [path], activeFile, unsavedFiles: [path]}` |
| `workspace/openFile` | `{path, line?, column?}` | `null` |
| `editor/active` | — | `{path, language, cursor, selection: {start, end}, selectedText, unsaved}` or `null` when no editor is active |
| `editor/text` | `{path?}` | text of the active editor, or of `path` (its open buffer with unsaved edits when open, else the file on disk) |
| `editor/edit` | `{path?, range?, text}` | `null`. Replaces `range` (default: the selection; an empty selection inserts at the cursor) as one undoable edit. With `path` the file is opened first if needed. |
| `editor/select` | `{path?, start, end?}` | `null`. Moves the cursor to `start` (scrolling to it) and selects up to `end`. |
| `ui/notify` | `{message, title?, level?: "info" \| "success" \| "warning" \| "error"}` | `null`. The title defaults to the extension name. |
| `ui/pick` | `{items: [string \| {label, detail?}], placeholder?}` | the chosen index, or `null` when cancelled |
| `ui/input` | `{title?, value?}` | the entered text, or `null` when cancelled |
| `terminal/run` | `{command, title?, cwd?}` | `null`. Runs `command` through the login shell in a terminal tab (kept after it exits, `file:line` links clickable). |

Edits made by extensions behave like typing: they mark the file unsaved, reach language servers and
can be undone with ⌘Z. Extensions cannot save, delete or write files through the API (they can, of
course, use the file system themselves).

## Events

Subscribe in the manifest; each arrives as `event` with `{type, path}`:

| Type | When |
|---|---|
| `file_opened` | a file gets an editor tab |
| `file_closed` | its tab closes |
| `file_saved` | it is written to disk from the IDE |
| `file_changed` | its buffer changes (every edit — keep the handler cheap; call `editor/text` for the content) |
| `active_editor_changed` | another editor tab becomes active |

Events are sent only to a running extension: files restored when the project opens may be reported
before an eager extension has finished starting. Call `workspace/info` after `initialize` to catch up.

## Versioning

`apiVersion` is `1`. Additions (new methods, params, events) keep the version; incompatible changes
bump it. Ignore unknown params and event types.
