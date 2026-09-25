# dank-pinentry socket protocol

The `dank-pinentry` binary talks to the DankMaterialShell plugin over a Unix
domain socket using newline-delimited JSON.

The **plugin listens**, the **binary connects**. That direction matters: the
plugin owns a stable well-known path and its lifetime is the shell's, while
the binary is spawned once per prompt by gpg-agent. If the connection fails,
the binary falls back to the terminal frontend rather than failing the prompt.

`dank-askpass` is a second client of the same protocol. It sends a single
`getpin`, `confirm` or `message` request (for ssh's `SSH_ASKPASS_PROMPT`
modes) with the caller's prompt as `description`, and has no fallback.

## Transport

| | |
|---|---|
| Default path | `$XDG_RUNTIME_DIR/dms-pinentry.sock` |
| Override | `DANK_PINENTRY_SOCKET_PATH`, or `socket_path` in `config.toml` |
| Framing | One JSON object per line, `\n`-terminated |
| Encoding | UTF-8 |
| Lifetime | One connection per prompt; closed after the reply |

### Why not `dms ipc call`

DMS exposes an IPC mechanism, but its arguments land in the process command
line and are visible in `/proc/<pid>/cmdline` to anything that can read it. A
passphrase must never travel that way. The socket is used for everything.

### Access control

* The socket lives in `$XDG_RUNTIME_DIR`, which is `0700`.
* The binary verifies via `SO_PEERCRED` that the listening process runs as the
  same UID before sending anything. This is the check that actually matters:
  the path alone is not a guarantee.
* Quickshell creates the socket `0755`. It is protected by the containing
  directory rather than its own mode, which is why the peer-credential check
  is not optional.

## Versioning

Every message carries `"v"`. The current version is `1`. A receiver that sees
an unknown version replies with an `error` message and closes, rather than
guessing at the payload.

## Messages

### Request: binary to plugin

Exactly one request per connection.

```json
{
  "v": 1,
  "type": "getpin",
  "id": "b3f1c2",
  "title": "Passphrase",
  "description": "Please enter the passphrase to unlock the OpenPGP secret key:\n\"Augusto Dias <a@example.com>\"",
  "prompt": "Passphrase:",
  "error": "Bad Passphrase",
  "ok": "OK",
  "cancel": "Cancel",
  "notok": null,
  "repeat": "Repeat:",
  "repeatError": "Passphrases do not match",
  "qualityBar": "Quality:",
  "keyinfo": "n/9A1B2C3D4E5F",
  "owner": { "pid": 44212, "uid": 1000, "host": "nixos", "command": "git commit -S" },
  "timeout": 60
}
```

`type` is one of:

| `type` | Meaning | Expected reply |
|---|---|---|
| `getpin` | Ask for a passphrase | `pin` or `cancel` |
| `confirm` | Ask a yes/no question | `confirm` or `cancel` |
| `message` | Show an informational message, one button | `confirm` |
| `ping` | Liveness/handshake check | `pong` |

All descriptive fields are optional and may be `null`. `description` contains
real newlines: it has already been percent-decoded from the Assuan wire form.

`owner` is derived from `OPTION owner=` plus a `/proc` lookup of the command
line. It lets the prompt say *which* process is asking, which stock pinentry
does not surface. It may be `null`.

`timeout` is in seconds: gpg-agent's `SETTIMEOUT` if it sent one, otherwise
the binary's `--timeout`, `DANK_PINENTRY_TIMEOUT` or config `timeout`,
otherwise 60. `0` means no timeout. The plugin should close the prompt when it
expires so a forgotten dialog cannot hold an exclusive keyboard grab forever.

### Reply: plugin to binary

Exactly one reply per connection.

```json
{ "v": 1, "type": "pin", "id": "b3f1c2", "pin": "correct horse battery staple", "repeated": true }
{ "v": 1, "type": "cancel", "id": "b3f1c2" }
{ "v": 1, "type": "confirm", "id": "b3f1c2", "outcome": "confirmed" }
{ "v": 1, "type": "error", "id": "b3f1c2", "message": "no display available" }
{ "v": 1, "type": "pong", "protocol": 1 }
```

`repeated` (default `false`) says the prompt had the user type the passphrase
twice and both entries matched. The binary then reports `S PIN_REPEATED` to
gpg-agent, which skips its own "re-enter this passphrase" prompt. It is only
honoured when the request carried `repeat`; a prompt that answers a `repeat`
request with a single field leaves it `false`, and gpg-agent asks again itself.

`outcome` is `confirmed`, `declined` or `cancelled`. These map onto gpg-error
values: `confirmed` to `OK`, `declined` to `NOT_CONFIRMED` (83886194), and
`cancelled` to `CANCELED` (83886179).

`id` echoes the request so a late reply from an abandoned prompt can be
discarded rather than answering the wrong request.

## Security notes

The passphrase crosses this socket in plaintext. That is unavoidable and is
also true of every other pinentry: the secret has to reach gpg-agent somehow.
What the socket adds over the alternatives is that it never appears in a
command line, an environment variable, or a file.

Two limitations are inherent to putting the UI in QML and are **not** fixed by
anything in this protocol:

1. **QML cannot wipe a passphrase.** JavaScript strings are immutable and
   garbage-collected, and `TextInput.text` leaves copies behind. The Rust side
   uses an `mlock`ed, zeroed-on-drop buffer; the QML side cannot.
2. **The shell outlives the prompt.** A conventional pinentry is a short-lived
   process whose pages are released on exit. Running the UI inside the
   long-lived shell means remnants can persist in its heap for the session.

Both are accepted trade-offs for shell integration. Users who object should
set `ui = "tty"`, which keeps the passphrase entirely inside the short-lived
binary and its locked buffer.
