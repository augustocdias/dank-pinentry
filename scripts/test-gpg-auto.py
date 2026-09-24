#!/usr/bin/env python3
"""Fully automated end-to-end test: real gpg-agent, real signature.

Drives the TTY frontend so no human has to type anything. A pty stands in for
the user's terminal; gpg-agent passes its path to dank-pinentry via
`OPTION ttyname`, exactly as it would in a real session.

Uses a throwaway GNUPGHOME, so the caller's keyring and agent are untouched.

Usage: scripts/test-gpg-auto.py
"""

import os
import pty
import selectors
import shutil
import subprocess
import sys
import tempfile
import time

PASSPHRASE = "integration-test-passphrase"
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BINARY = os.path.join(REPO, "target", "release", "dank-pinentry")


def run(cmd, env, **kw):
    return subprocess.run(cmd, env=env, capture_output=True, text=True, **kw)


def main():
    if not os.path.exists(BINARY):
        print(f"error: {BINARY} not built (cargo build --release)", file=sys.stderr)
        return 1

    home = tempfile.mkdtemp(prefix="dank-pinentry-gpg.")
    os.chmod(home, 0o700)

    env = dict(os.environ)
    env["GNUPGHOME"] = home
    env["DANK_PINENTRY_UI"] = "tty"

    with open(os.path.join(home, "gpg-agent.conf"), "w") as fh:
        fh.write(f"pinentry-program {BINARY}\n")
        # No caching, so the signing step genuinely reaches pinentry.
        fh.write("default-cache-ttl 0\nmax-cache-ttl 0\n")

    try:
        print(f"GNUPGHOME={home}")
        print("==> generating a test key")
        res = run(
            [
                "gpg", "--batch", "--yes", "--pinentry-mode", "loopback",
                "--passphrase", PASSPHRASE,
                "--quick-generate-key", "Pinentry Dank Test <test@example.invalid>",
                "default", "default", "never",
            ],
            env,
        )
        if res.returncode != 0:
            print("FAIL: key generation failed", file=sys.stderr)
            print(res.stderr[-2000:], file=sys.stderr)
            return 1
        print("    key created")

        run(["gpgconf", "--reload", "gpg-agent"], env)

        # A pty stands in for the user's terminal. gpg reads GPG_TTY and hands
        # it to the agent, which passes it to pinentry as OPTION ttyname.
        master, slave = pty.openpty()
        env["GPG_TTY"] = os.ttyname(slave)
        print(f"==> signing with GPG_TTY={env['GPG_TTY']}")

        sig = os.path.join(home, "out.sig")
        proc = subprocess.Popen(
            ["gpg", "--batch", "--yes", "--sign", "--output", sig],
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        proc.stdin.write(b"integration test payload\n")
        proc.stdin.close()
        # Python < 3.13 communicate() flushes stdin even when it is closed.
        proc.stdin = None

        # Wait for the prompt to be drawn on the pty before answering, so we
        # are genuinely responding to pinentry rather than racing it.
        sel = selectors.DefaultSelector()
        sel.register(master, selectors.EVENT_READ)
        screen = b""
        deadline = time.time() + 20
        while time.time() < deadline:
            if sel.select(timeout=0.5):
                screen += os.read(master, 4096)
                if b"assphrase" in screen or b"PIN" in screen:
                    break

        if not screen:
            proc.kill()
            print("FAIL: pinentry never drew a prompt on the tty", file=sys.stderr)
            return 1

        print(f"    prompt drawn ({len(screen)} bytes)")
        os.write(master, PASSPHRASE.encode() + b"\r")

        try:
            _out, err = proc.communicate(timeout=30)
        except subprocess.TimeoutExpired:
            proc.kill()
            print("FAIL: gpg hung after the passphrase was entered", file=sys.stderr)
            return 1

        sel.close()
        os.close(master)
        os.close(slave)

        if proc.returncode != 0:
            print("FAIL: signing failed", file=sys.stderr)
            print(err.decode(errors="replace")[-2000:], file=sys.stderr)
            return 1

        # The passphrase must never have been echoed to the terminal.
        if PASSPHRASE.encode() in screen:
            print("FAIL: passphrase was echoed on the tty", file=sys.stderr)
            return 1
        print("    passphrase was not echoed")

        print("==> verifying the signature")
        res = run(["gpg", "--verify", sig], env)
        if res.returncode != 0:
            print("FAIL: signature does not verify", file=sys.stderr)
            print(res.stderr[-2000:], file=sys.stderr)
            return 1

        print()
        print("PASS: real gpg-agent produced and verified a signature via dank-pinentry")
        return 0

    finally:
        run(["gpgconf", "--kill", "gpg-agent"], env)
        shutil.rmtree(home, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
