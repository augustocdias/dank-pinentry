#!/usr/bin/env python3
"""Drive the TTY frontend through a real pty.

The terminal frontend cannot be exercised by piping stdin: it deliberately
opens the device named by `OPTION ttyname` instead, precisely so it does not
collide with the Assuan stream on stdin/stdout. This harness allocates a pty,
hands its path to the binary, and types into the master side.

Usage: scripts/test-tty.py [path-to-binary]
"""

import os
import pty
import re
import selectors
import subprocess
import sys
import time

BINARY = sys.argv[1] if len(sys.argv) > 1 else "./target/debug/dank-pinentry"

PASSPHRASE = "hunter2"


def drain(fd, sel, timeout=0.4):
    """Read whatever is available on a fd within the timeout."""
    chunks = []
    deadline = time.time() + timeout
    while time.time() < deadline:
        remaining = deadline - time.time()
        for _key, _mask in sel.select(timeout=remaining):
            try:
                data = os.read(fd, 4096)
            except OSError:
                return b"".join(chunks)
            if not data:
                return b"".join(chunks)
            chunks.append(data)
    return b"".join(chunks)


def run_case(name, script, keystrokes, expect):
    master, slave = pty.openpty()
    ttyname = os.ttyname(slave)

    proc = subprocess.Popen(
        [BINARY],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={**os.environ, "DANK_PINENTRY_UI": "tty"},
    )

    sel = selectors.DefaultSelector()
    sel.register(master, selectors.EVENT_READ)

    full_script = script.replace("{TTY}", ttyname)
    proc.stdin.write(full_script.encode())
    proc.stdin.flush()

    # Let the prompt render before typing into it.
    time.sleep(0.3)
    rendered = drain(master, sel)

    for key in keystrokes:
        os.write(master, key)
        time.sleep(0.05)

    time.sleep(0.2)
    rendered += drain(master, sel)

    proc.stdin.write(b"BYE\n")
    proc.stdin.flush()

    try:
        stdout, stderr = proc.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        stdout, stderr = proc.communicate()
        print(f"  FAIL {name}: binary hung")
        return False

    sel.close()
    os.close(master)
    os.close(slave)

    protocol = stdout.decode(errors="replace")
    screen = rendered.decode(errors="replace")

    ok = expect(protocol, screen)
    status = "ok  " if ok else "FAIL"
    print(f"  {status} {name}")
    if not ok:
        print(f"       protocol: {protocol!r}")
        print(f"       screen:   {screen!r}")
        if stderr:
            print(f"       stderr:   {stderr.decode(errors='replace')!r}")
    return ok


def main():
    if not os.path.exists(BINARY):
        print(f"binary not found: {BINARY}", file=sys.stderr)
        return 1

    print("TTY frontend")
    results = []

    results.append(
        run_case(
            "typed passphrase is returned on a D line",
            "OPTION ttyname={TTY}\n"
            "SETDESC Unlock%20the%20key\n"
            "SETPROMPT Passphrase:\n"
            "GETPIN\n",
            [PASSPHRASE.encode(), b"\r"],
            lambda proto, screen: f"D {PASSPHRASE}\n" in proto,
        )
    )

    results.append(
        run_case(
            "description and prompt are drawn on the tty",
            "OPTION ttyname={TTY}\nSETDESC Unlock%20the%20key\nSETPROMPT Passphrase:\nGETPIN\n",
            [PASSPHRASE.encode(), b"\r"],
            lambda proto, screen: "Unlock the key" in screen and "Passphrase:" in screen,
        )
    )

    results.append(
        run_case(
            "input is masked, never echoed",
            "OPTION ttyname={TTY}\nSETPROMPT Passphrase:\nGETPIN\n",
            [PASSPHRASE.encode(), b"\r"],
            lambda proto, screen: PASSPHRASE not in screen and "*" in screen,
        )
    )

    results.append(
        run_case(
            "escape cancels",
            "OPTION ttyname={TTY}\nGETPIN\n",
            [b"abc", b"\x1b"],
            lambda proto, screen: "ERR 83886179" in proto,
        )
    )

    results.append(
        run_case(
            "ctrl-c cancels",
            "OPTION ttyname={TTY}\nGETPIN\n",
            [b"abc", b"\x03"],
            lambda proto, screen: "ERR 83886179" in proto,
        )
    )

    results.append(
        run_case(
            "backspace removes a character",
            "OPTION ttyname={TTY}\nGETPIN\n",
            [b"abcX", b"\x7f", b"\r"],
            lambda proto, screen: "D abc\n" in proto,
        )
    )

    results.append(
        run_case(
            "ctrl-u clears the whole entry",
            "OPTION ttyname={TTY}\nGETPIN\n",
            [b"garbage", b"\x15", b"real", b"\r"],
            lambda proto, screen: "D real\n" in proto,
        )
    )

    results.append(
        run_case(
            "multibyte input survives round trip",
            "OPTION ttyname={TTY}\nGETPIN\n",
            ["pässwörd".encode(), b"\r"],
            lambda proto, screen: "D pässwörd\n" in proto,
        )
    )

    results.append(
        run_case(
            "multibyte input counts as one mask character each",
            "OPTION ttyname={TTY}\nGETPIN\n",
            ["üüü".encode(), b"\r"],
            # Three characters typed must draw three asterisks, not six.
            lambda proto, screen: re.search(r"\*{3}(?!\*)", screen) is not None,
        )
    )

    results.append(
        run_case(
            "empty passphrase is accepted",
            "OPTION ttyname={TTY}\nGETPIN\n",
            [b"\r"],
            lambda proto, screen: "D \n" in proto or "D\n" in proto,
        )
    )

    results.append(
        run_case(
            "confirm accepts y",
            "OPTION ttyname={TTY}\nSETDESC Really%3F\nCONFIRM\n",
            [b"y"],
            lambda proto, screen: "ERR" not in proto,
        )
    )

    results.append(
        run_case(
            "confirm declines with n",
            "OPTION ttyname={TTY}\nSETDESC Really%3F\nCONFIRM\n",
            [b"n"],
            lambda proto, screen: "ERR 83886194" in proto,
        )
    )

    results.append(
        run_case(
            "message is dismissed by any key",
            "OPTION ttyname={TTY}\nSETDESC Insert%20card\nMESSAGE\n",
            [b" "],
            lambda proto, screen: "ERR" not in proto,
        )
    )

    results.append(
        run_case(
            "prompt is erased from the terminal afterwards",
            "OPTION ttyname={TTY}\nSETDESC Secret%20thing\nGETPIN\n",
            [PASSPHRASE.encode(), b"\r"],
            # The erase sequence must be emitted so the shell is left clean.
            lambda proto, screen: "\x1b[K" in screen,
        )
    )

    passed = sum(results)
    total = len(results)
    print(f"\n{passed}/{total} passed")
    return 0 if passed == total else 1


if __name__ == "__main__":
    sys.exit(main())
