# dank-pinentry

A pinentry that draws its prompt either as an inline masked prompt on your
terminal, or as a dialog inside [DankMaterialShell][dms].

 https://github.com/user-attachments/assets/dcfea9bc-3341-4aac-bf41-0a62731d3914

```
gpg-agent ──stdin/stdout (Assuan)──> dank-pinentry
                                          │
                        ┌─────────────────┴─────────────────┐
                   TTY frontend                       DMS frontend
            raw termios on the device            unix socket client →
            named by OPTION ttyname              SocketServer in the
                                                 DankMaterialShell plugin
```

By default the terminal wins whenever one is usable, and the shell dialog is
used otherwise. So `git commit -S` in a terminal prompts you in that terminal,
while an SSH key needed by a background job gets a dialog.

## Screenshots

With the default `bar` placement, prompts replace the widget in DankBar:

|                         |                                                                                |
| ----------------------- | ------------------------------------------------------------------------------ |
| Passphrase              | ![Empty passphrase field](screenshots/input.png)                               |
| Typing                  | ![Masked passphrase field](screenshots/input_filled.png)                       |
| Retry, with bar text on | ![Bad passphrase error, description and requester](screenshots/bad_phrase.png) |
| Message from gpg-agent  | ![Message with an OK button](screenshots/confirm.png)                          |
| Waiting to be opened    | ![Key badge](screenshots/key.png)                                              |

## Status

Working: the Assuan protocol layer, the TTY frontend, frontend selection, and
the DMS plugin (passphrase, confirm and message prompts, placement, focus and
notification settings). Verified end to end against a real gpg-agent, from key
generation through a verified signature.

Not implemented and not planned: the passphrase-quality bar (`INQUIRE QUALITY`), the
caps-lock hint, and the external password cache.

## AI notice

This was mostly written with an AI coding assistant. I've reviewed it and use
it daily on Hyprland, and the protocol layer is tested end to end against a
real gpg-agent, but I built it for my own setup. **I take no responsibility for
anything that happens if you decide to install it; there is no warranty (see
[LICENSE](LICENSE)).** It handles your passphrases, so read the
[Security](#security) section before installing. The translations are
machine-written and have not been reviewed by native speakers.

## Install

### Nix

```nix
{
  inputs.dank-pinentry.url = "github:augustocdias/dank-pinentry";

  # in your home-manager configuration
  imports = [inputs.dank-pinentry.homeModules.default];

  programs.dank-pinentry = {
    enable = true;
    ui = "auto";          # "auto" | "tty" | "dms"
  };
}
```

Then point gpg-agent at it. Most configurations already set this, which is why
the module leaves it alone by default:

```
pinentry-program /path/to/dank-pinentry
```

and reload: `gpgconf --reload gpg-agent`.

### Other distributions (from source)

No prebuilt binaries are published, so the binary is built from source. It
needs Rust 1.85 or newer (install it with [rustup](https://rustup.rs) if your
distribution ships something older) and a C linker, which comes with your
distribution's base development tools (`base-devel` on Arch, `build-essential`
on Debian/Ubuntu, `gcc` on Fedora). There are no other build dependencies.

**1. Build and install the binary.**

```sh
git clone https://github.com/augustocdias/dank-pinentry
cd dank-pinentry
cargo build --release
sudo install -Dm755 target/release/dank-pinentry /usr/local/bin/dank-pinentry
```

Without root, install to `~/.local/bin/dank-pinentry` instead. The plugin
finds the binary on `PATH`, in `~/.local/bin` or in `/usr/local/bin`.

**2. Point gpg-agent at it.** Add to `~/.gnupg/gpg-agent.conf`, replacing any
existing `pinentry-program` line. The path must be absolute; gpg-agent does not
expand `~`:

```
pinentry-program /usr/local/bin/dank-pinentry
```

Then restart the agent so it picks up the change:

```sh
gpgconf --kill gpg-agent
```

At this point prompts already work in a terminal.

**3. Install the plugin.** Once the plugin is listed in the DMS plugin
registry, install it from Settings → Plugins. Until then, copy it from the
checkout:

```sh
mkdir -p ~/.config/DankMaterialShell/plugins
cp -r plugin ~/.config/DankMaterialShell/plugins/dankbarPinentry
```

(Use `ln -s "$PWD/plugin" …` instead if you want to track the checkout.)

**4. Enable it and add the widget.** Enable DankBar Pinentry in Settings →
Plugins (or `dms ipc call plugins enable dankbarPinentry`), then add the Dank
Pinentry widget to a horizontal bar: with the default `bar` placement that is
where prompts appear. DMS 1.7 or newer is required.

**Updating:** `git pull`, then repeat steps 1 and 3. **Uninstalling:** remove
the binary and `~/.config/DankMaterialShell/plugins/dankbarPinentry`, and restore
your previous `pinentry-program` line.

If DMS's plugin settings are managed by Nix, `plugin_settings.json` is
read-only and an in-shell enable lasts only until DMS restarts; enable the
plugin through your DMS Nix configuration instead (see below).

## Configuration

The binary and the plugin are configured separately, because they run in
different processes and, on a declaratively managed system, from different
sources.

### Binary: `$XDG_CONFIG_HOME/dank-pinentry/config.toml`

| Key           | Values               | Default                              | Meaning                                                     |
| ------------- | -------------------- | ------------------------------------ | ----------------------------------------------------------- |
| `ui`          | `auto`, `tty`, `dms` | `auto`                               | Which frontend to use                                       |
| `socket_path` | path                 | `$XDG_RUNTIME_DIR/dms-pinentry.sock` | Where the plugin listens                                    |
| `mask_char`   | character            | `*`                                  | Drawn per typed character in the terminal                   |
| `timeout`     | seconds              | `60`                                 | Cancel an unanswered prompt after this long; `0` never does |

Each key can be overridden with `DANK_PINENTRY_<KEY>` (e.g.
`DANK_PINENTRY_TIMEOUT`), and `ui` and `timeout` also with `--ui` and
`--timeout`. Precedence is: command line, then environment, then config file.

The timeout applies to every prompt, including one waiting behind the bar
badge. gpg-agent's own `pinentry-timeout` option, if set in `gpg-agent.conf`,
overrides all of the above: it is sent over the protocol after startup.

### Plugin: DMS plugin settings

Placement and every visual/focus behaviour are independent toggles.

| Setting       | Values                           | Default           | Meaning                                                                                                                    |
| ------------- | -------------------------------- | ----------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `placement`   | `bar`, `center`, `top`, `bottom` | `bar`             | Inside the bar, a dialog in the middle, or a strip against an edge                                                         |
| `barText`     | bool                             | `false`           | With `bar`: also show the description, errors and requester                                                                |
| `dimBackdrop` | bool                             | `false`           | Darken the screen behind the prompt; ignored with `bar`                                                                    |
| `focusMode`   | `take`, `hold`, `leave`          | `take`            | Take the keyboard; take it and keep other windows from getting it back; or leave it where it is until you click the prompt |
| `autoOpen`    | bool                             | `true`            | Show on arrival, rather than waiting behind the bar badge                                                                  |
| `notify`      | bool                             | `true`            | Send a notification when a passphrase is needed                                                                            |
| `notifyIcon`  | icon name or path                | `dialog-password` | Icon on that notification                                                                                                  |
| `showOwner`   | bool                             | `true`            | Name the process asking                                                                                                    |
| `timeoutRing` | bool                             | `true`            | Show the countdown to expiry                                                                                               |

With `bar`, every prompt appears in place of the DankBar Pinentry widget, sized
from the bar's height; add that widget to a horizontal bar. A passphrase prompt
is a bare masked field. A confirmation shows its question with gpg-agent's
buttons, and a message its text with OK; long text is cut to one line, with
the full text on hover. Choosing a new passphrase uses the same single field:
gpg-agent then asks you to re-enter it.

If no horizontal bar has the widget, the prompt waits, with a warning, until
one does or the prompt times out. DMS gives plugins no API for bar keyboard
focus, so the widget briefly overrides its own bar's focus while prompting.

Escape cancels in every focus mode. With `leave`, clicking the prompt (or the
IPC `open` below) gives it the keyboard. In the bar, Enter picks the selected
button (initially OK), and Tab or the arrow keys move between buttons.

On NixOS/home-manager the shared plugin settings file is a read-only store
symlink, so the in-shell toggles cannot be saved. Set these in your DMS
configuration instead:

```nix
plugins.dankbarPinentry = {
  enable = true;
  src = "${inputs.dank-pinentry}/plugin";
  settings = {
    placement = "bar";
    barText = true;
    focusMode = "take";
  };
};
```

The attribute must be named `dankbarPinentry`, the plugin's id: DMS looks up the
`enabled` flag by id. `programs.dank-pinentry.installPlugin` sets `src` for
you.

### IPC

For key bindings, e.g. to open a prompt waiting behind the badge without
reaching for the mouse:

| Command                            | Effect                                                                              |
| ---------------------------------- | ----------------------------------------------------------------------------------- |
| `dms ipc call dankbarPinentry open`   | Open the waiting prompt, or give the keyboard to the one on screen                  |
| `dms ipc call dankbarPinentry cancel` | Cancel the current prompt                                                           |
| `dms ipc call dankbarPinentry status` | `idle`, `pending` (behind the badge), `waiting` (no widget to show it in) or `open` |

```
bind = SUPER, P, exec, dms ipc call dankbarPinentry open
```

## Which process is asking?

gpg-agent tells a pinentry the pid of the process on whose behalf it is
asking. `dank-pinentry` resolves that to a command line and shows it:

> Requested by `git commit -S`

Stock pinentry largely ignores this, which means any process can produce a
passphrase box indistinguishable from a genuine one. Naming the requester
makes a spoofed prompt easier to spot. Turn it off with `showOwner = false`.

## Security

The Rust side keeps the passphrase in an `mlock`ed buffer that is zeroed on
drop, and never writes it anywhere except the `D` line back to gpg-agent. The
socket to the plugin is checked with `SO_PEERCRED` so a socket planted by
another user is refused.

Two limitations are inherent to drawing the prompt in QML, and are stated
plainly rather than papered over:

1. **QML cannot wipe a passphrase.** JavaScript strings are immutable and
   garbage-collected, and `TextInput.text` leaves copies behind.
1. **The shell outlives the prompt.** A conventional pinentry is a short-lived
   process whose memory is released when it exits. Running the UI inside the
   long-lived shell means remnants can persist in its heap for the session.

If that trade-off is unacceptable, set `ui = "tty"`. The terminal frontend
keeps the passphrase entirely inside the short-lived binary and its locked
buffer.

`focusMode = "hold"` deserves its own warning: it requests exclusive keyboard
focus, so if the shell stops responding while a prompt is open, the keyboard
stays captured until the prompt times out.

## Development

```
cargo test                          # unit + transcript tests
python3 scripts/test-tty.py         # TTY frontend, driven through a pty
python3 scripts/test-gpg-auto.py    # full end-to-end, real gpg-agent, no input needed
./scripts/test-gpg-integration.sh dms   # interactive, exercises the DMS dialog
```

The integration scripts use a throwaway `GNUPGHOME`, so your own keyring and
agent are never touched.

For the plugin, symlink it into the DMS plugins directory and reload:

```
dms ipc call plugins reload dankbarPinentry
journalctl --user -f | rg dankbarPinentry
```

Two DMS quirks worth knowing:

- The startup-check component is created without a cache-busting query
  string, so the QML engine caches it for the life of the shell. Editing
  `DependencyCheck.qml` has no effect until you restart
  (`systemctl --user restart dms.service`) or rename the file.
- Manifests are re-read based on mtime, so a stale `plugin.json` can survive a
  scan.

## Translations

The plugin ships translations for the 21 languages DMS supports, in
`plugin/translations/<locale>.json`. Every user-facing string goes through
`I18n.trFor("dankbarPinentry", "…")`; the id must stay a literal, because DMS's
extraction tooling reads call sites. CI rejects plain `I18n.tr(`, translation
keys that no longer match a string in the QML, and dropped `%1` placeholders.

## Protocol

The socket protocol between the binary and the plugin is documented in
[PROTOCOL.md](PROTOCOL.md).

[dms]: https://github.com/AvengeMedia/DankMaterialShell
