#!/usr/bin/env python3
"""Drives dank-askpass against a fake plugin socket.

Usage: test-askpass.py [path/to/dank-askpass]
"""

import json
import os
import socket
import subprocess
import sys
import tempfile
import threading

BINARY = sys.argv[1] if len(sys.argv) > 1 else "target/release/dank-askpass"


class FakePlugin:
    """Answers pings, then replies to each prompt with `answer(request)`."""

    def __init__(self, path, answer):
        self.path = path
        self.answer = answer
        self.requests = []
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.bind(path)
        self.sock.listen()
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def serve(self):
        while True:
            try:
                conn, _ = self.sock.accept()
            except OSError:
                return
            with conn:
                line = conn.makefile().readline()
                if not line:
                    continue
                request = json.loads(line)
                if request["type"] == "ping":
                    reply = {"v": 1, "type": "pong", "protocol": 1}
                else:
                    self.requests.append(request)
                    reply = self.answer(request)
                conn.sendall((json.dumps(reply) + "\n").encode())

    def close(self):
        self.sock.close()


def run(socket_path, args, env_extra=None):
    env = dict(os.environ, DANK_PINENTRY_SOCKET_PATH=socket_path)
    env.pop("SSH_ASKPASS_PROMPT", None)
    env.update(env_extra or {})
    return subprocess.run(
        [BINARY, *args], env=env, capture_output=True, timeout=10, check=False
    )


def main():
    failures = []

    def check(name, cond):
        print(f"{'ok  ' if cond else 'FAIL'} {name}")
        if not cond:
            failures.append(name)

    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "plugin.sock")

        result = run(path, [])
        check("no prompt argument exits non-zero", result.returncode != 0)

        result = run(path, ["[sudo] password for me: "])
        check("missing plugin exits non-zero", result.returncode != 0)
        check("missing plugin prints nothing on stdout", result.stdout == b"")

        def answer(request):
            rid = request["id"]
            if request["type"] == "getpin":
                return {"v": 1, "type": "pin", "id": rid, "pin": "hunter2"}
            if request["type"] == "confirm":
                outcome = "confirmed" if "yes" in request["description"] else "declined"
                return {"v": 1, "type": "confirm", "id": rid, "outcome": outcome}
            return {"v": 1, "type": "confirm", "id": rid, "outcome": "confirmed"}

        plugin = FakePlugin(path, answer)
        try:
            result = run(path, ["[sudo] password for me: "])
            check("passphrase exits zero", result.returncode == 0)
            check("passphrase is printed with a newline", result.stdout == b"hunter2\n")
            request = plugin.requests[-1]
            check("getpin request sent", request["type"] == "getpin")
            check(
                "prompt becomes the description",
                request.get("description") == "[sudo] password for me:",
            )
            check("owner is the caller", request.get("owner", {}).get("pid") == os.getpid())

            result = run(path, ["allow yes?"], {"SSH_ASKPASS_PROMPT": "confirm"})
            check("confirmed exits zero", result.returncode == 0)
            check("confirm prints nothing", result.stdout == b"")
            result = run(path, ["allow no?"], {"SSH_ASKPASS_PROMPT": "confirm"})
            check("declined exits non-zero", result.returncode != 0)

            result = run(path, ["touch your key"], {"SSH_ASKPASS_PROMPT": "none"})
            check("notification exits zero", result.returncode == 0)
            check("notification sends a message", plugin.requests[-1]["type"] == "message")
        finally:
            plugin.close()

        os.unlink(path)
        plugin = FakePlugin(path, lambda r: {"v": 1, "type": "cancel", "id": r["id"]})
        try:
            result = run(path, ["password: "])
            check("cancel exits non-zero", result.returncode != 0)
            check("cancel prints nothing", result.stdout == b"")
        finally:
            plugin.close()

    if failures:
        print(f"\n{len(failures)} failed", file=sys.stderr)
        return 1
    print("\nall askpass checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
