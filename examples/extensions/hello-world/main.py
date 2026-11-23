"""Example Power IDEA extension (Python 3.8+, standard library only).

The IDE starts this script with stdin/stdout connected to it and speaks JSON-RPC 2.0
with LSP-style `Content-Length` framing. stdout belongs to the protocol: log to stderr
(it ends up in ~/Library/Logs/Power IDEA/extensions/hello-world.log).

The `Connection` class is a tiny, reusable client; the handlers below show each API.
"""

import itertools
import json
import sys
import threading


class RpcError(Exception):
    pass


class Connection:
    """Reads IDE messages on the main thread; handlers run on worker threads so they
    can call back into the IDE (`request`) while the reader keeps going."""

    def __init__(self):
        self._ids = itertools.count(1)
        self._pending = {}
        self._lock = threading.Lock()
        self._write_lock = threading.Lock()
        self.commands = {}
        self.events = {}

    # -- calling the IDE ---------------------------------------------------------------

    def request(self, method, params=None):
        """Sends a request to the IDE and waits for its result."""
        msg_id = "ext-{}".format(next(self._ids))
        done = threading.Event()
        slot = {}
        with self._lock:
            self._pending[msg_id] = (done, slot)
        self._send({"jsonrpc": "2.0", "id": msg_id, "method": method, "params": params or {}})
        done.wait()
        if "error" in slot:
            raise RpcError(slot["error"].get("message", "request failed"))
        return slot.get("result")

    # -- plumbing ------------------------------------------------------------------------

    def _send(self, message):
        body = json.dumps(message).encode("utf-8")
        with self._write_lock:
            sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body))
            sys.stdout.buffer.write(body)
            sys.stdout.buffer.flush()

    def _read(self):
        length = None
        while True:
            line = sys.stdin.buffer.readline()
            if not line:
                return None
            line = line.strip()
            if not line:
                if length is not None:
                    break
                continue
            if line.lower().startswith(b"content-length:"):
                length = int(line.split(b":", 1)[1])
        return json.loads(sys.stdin.buffer.read(length).decode("utf-8"))

    def _answer(self, msg_id, fn, params):
        try:
            self._send({"jsonrpc": "2.0", "id": msg_id, "result": fn(params)})
        except Exception as err:  # report any handler failure to the IDE
            self._send({"jsonrpc": "2.0", "id": msg_id, "error": {"code": -32603, "message": str(err)}})

    def _execute(self, params):
        handler = self.commands.get(params.get("command"))
        if handler is None:
            raise RpcError("unknown command {}".format(params.get("command")))
        return handler()

    def run(self):
        while True:
            message = self._read()
            if message is None:
                return
            method, msg_id = message.get("method"), message.get("id")
            params = message.get("params") or {}
            if method is None:  # an answer to one of our requests
                with self._lock:
                    done, slot = self._pending.pop(msg_id, (None, None))
                if done:
                    slot.update(message)
                    done.set()
            elif method == "initialize":
                log("initialized for", params["project"]["root"], "api", params["apiVersion"])
                self._send({"jsonrpc": "2.0", "id": msg_id, "result": {}})
            elif method == "command/execute":
                threading.Thread(target=self._answer, args=(msg_id, self._execute, params), daemon=True).start()
            elif method == "event":
                handler = self.events.get(params.get("type"))
                if handler:
                    threading.Thread(target=handler, args=(params,), daemon=True).start()
            elif method == "shutdown":
                return
            elif msg_id is not None:
                self._send({"jsonrpc": "2.0", "id": msg_id,
                            "error": {"code": -32601, "message": "unknown method " + method}})


def log(*parts):
    print(*parts, file=sys.stderr, flush=True)


ide = Connection()


def command(name):
    def register(fn):
        ide.commands[name] = fn
        return fn
    return register


@command("hello-world.greet")
def greet():
    info = ide.request("workspace/info")
    ide.request("ui/notify", {
        "message": "Hello from {}! {} file(s) open.".format(info["name"], len(info["openFiles"])),
        "level": "success",
    })


@command("hello-world.upper-case")
def upper_case():
    editor = ide.request("editor/active")
    if editor is None or not editor["selectedText"]:
        ide.request("ui/notify", {"message": "Select some text first.", "level": "warning"})
        return
    ide.request("editor/edit", {"text": editor["selectedText"].upper()})


@command("hello-world.insert-header")
def insert_header():
    editor = ide.request("editor/active")
    if editor is None:
        return
    start = {"line": 1, "column": 1}
    ide.request("editor/edit", {
        "range": {"start": start, "end": start},
        "text": "// {}\n".format(editor["path"]),
    })


@command("hello-world.pick-file")
def pick_file():
    files = ide.request("workspace/info")["openFiles"]
    items = [{"label": path.rsplit("/", 1)[-1], "detail": path} for path in files]
    index = ide.request("ui/pick", {"items": items, "placeholder": "Open file"})
    if index is not None:
        ide.request("workspace/openFile", {"path": files[index], "line": 1})


@command("hello-world.ask-name")
def ask_name():
    name = ide.request("ui/input", {"title": "What is your name?", "value": "World"})
    if name:
        ide.request("ui/notify", {"message": "Nice to meet you, {}!".format(name)})


@command("hello-world.git-log")
def git_log():
    ide.request("terminal/run", {"command": "git log --oneline -20", "title": "Recent commits"})


def on_save(event):
    log("saved", event["path"])


ide.events["file_saved"] = on_save

if __name__ == "__main__":
    ide.run()
