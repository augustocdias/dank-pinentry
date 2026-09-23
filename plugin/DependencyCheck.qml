pragma ComponentBehavior: Bound

import QtQuick
import qs.Common

/**
 * Refuses to load without the binary, which would otherwise leave the plugin
 * sitting on a socket nothing connects to.
 *
 * When iterating: DMS builds this component without a cache-busting query
 * string, so the QML engine caches it for the life of the shell. Edits need a
 * shell restart, or rename the file.
 */
QtObject {
    /**
     * PATH alone is not enough: the shell inherits the session environment,
     * which on NixOS is entirely read-only store paths.
     */
    readonly property var extraPaths: ["$HOME/.local/bin", "/usr/local/bin", "/usr/bin"]

    function check(done) {
        const probes = ["command -v dank-pinentry >/dev/null 2>&1"];
        for (var i = 0; i < extraPaths.length; i++)
            probes.push("test -x \"" + extraPaths[i] + "/dank-pinentry\"");

        Proc.runCommand("dankbarPinentry.depCheck", ["sh", "-c", probes.join(" || ")], (stdout, exitCode) => {
            if (exitCode === 0) {
                done(null);
                return;
            }
            done({
                title: I18n.trFor("dankbarPinentry", "%1 is required").arg("dank-pinentry"),
                details: I18n.trFor("dankbarPinentry", "The %1 binary was not found on PATH, in ~/.local/bin or in /usr/local/bin. This plugin only draws the prompt; gpg-agent talks to the binary. Install it and set it as pinentry-program in ~/.gnupg/gpg-agent.conf, then run: %2").arg("dank-pinentry").arg("gpgconf --reload gpg-agent") + "\n\nhttps://github.com/augustocdias/dank-pinentry"
            });
        });
    }
}
