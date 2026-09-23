pragma ComponentBehavior: Bound

import QtQuick
import Quickshell
import Quickshell.Io
import qs.Common
import qs.Services
import qs.Modules.Plugins

/**
 * Listens for prompt requests from the dank-pinentry binary. The plugin is the
 * socket server; the binary is spawned once per prompt. See PROTOCOL.md.
 *
 * Requests are queued: SSH and GPG prompts can overlap, and two of them
 * fighting over an exclusive keyboard grab is not recoverable.
 */
PluginComponent {
    id: root

    readonly property int protocolVersion: 1
    readonly property string socketPath: (Quickshell.env("XDG_RUNTIME_DIR") || "/tmp") + "/dms-pinentry.sock"

    // Read-only at runtime; on NixOS these come from home-manager.
    readonly property string placement: pluginData.placement ?? "bar"
    readonly property bool dimBackdrop: pluginData.dimBackdrop ?? false
    /** "take", "hold" or "leave". */
    readonly property string focusMode: pluginData.focusMode ?? "take"
    readonly property bool barText: pluginData.barText ?? false
    readonly property bool notify: pluginData.notify ?? true
    readonly property bool autoOpen: pluginData.autoOpen ?? true
    readonly property bool showOwner: pluginData.showOwner ?? true
    readonly property bool timeoutRing: pluginData.timeoutRing ?? true
    readonly property string notifyIcon: pluginData.notifyIcon ?? "dialog-password"

    /** Entries of { socket, request, answered }. */
    property var queue: []
    property var active: null
    /** Pending but not shown; the user must open it from the bar. */
    property bool deferred: false

    /** Bar widget instances, which register themselves. */
    property var barWidgets: []
    /** The widget currently showing the prompt, or null. */
    property var barHost: null
    /** A bar prompt has no widget to show in yet; it waits for one. */
    property bool awaitingHost: false
    property bool pendingClaim: false

    /** -1 when the prompt has no deadline. */
    property int secondsRemaining: -1

    signal promptOpened
    signal promptClosed

    function log(msg) {
        console.info("[dankbarPinentry] " + msg);
    }

    function warn(msg) {
        console.warn("[dankbarPinentry] " + msg);
    }

    function send(socket, obj) {
        if (!socket) {
            warn("cannot reply: socket is gone");
            return false;
        }
        try {
            socket.write(JSON.stringify(obj) + "\n");
            socket.flush();
            return true;
        } catch (e) {
            warn("failed to write reply: " + e);
            return false;
        }
    }

    function handleLine(socket, line) {
        if (!line || line.trim().length === 0)
            return;

        let msg = null;
        try {
            msg = JSON.parse(line);
        } catch (e) {
            send(socket, {
                v: root.protocolVersion,
                type: "error",
                message: "invalid json"
            });
            return;
        }

        if (msg.v !== undefined && msg.v !== root.protocolVersion) {
            send(socket, {
                v: root.protocolVersion,
                type: "error",
                id: msg.id,
                message: "unsupported protocol version " + msg.v
            });
            return;
        }

        switch (msg.type) {
        case "ping":
            send(socket, {
                v: root.protocolVersion,
                type: "pong",
                protocol: root.protocolVersion
            });
            break;
        case "getpin":
        case "confirm":
        case "message":
            enqueue(socket, msg);
            break;
        default:
            send(socket, {
                v: root.protocolVersion,
                type: "error",
                id: msg.id,
                message: "unknown request type: " + msg.type
            });
            break;
        }
    }

    function enqueue(socket, request) {
        const next = root.queue.slice();
        next.push({
            socket: socket,
            request: request,
            answered: false
        });
        root.queue = next;

        log("queued " + request.type + " (" + root.queue.length + " pending)");

        if (!root.active)
            activateNext();
    }

    function activateNext() {
        if (root.active || root.queue.length === 0)
            return;

        const next = root.queue[0];
        root.active = next;
        // Deferred prompts expire too, so a forgotten badge does not keep
        // gpg-agent waiting.
        startTimeout(next.request);

        if (root.autoOpen)
            presentPrompt(next, null, false);
        else
            deferPrompt(next);
    }

    function pickBarWidget(preferred) {
        const capable = root.barWidgets.filter(w => w && w.canHostPrompt);
        if (preferred && capable.indexOf(preferred) !== -1)
            return preferred;
        const focused = CompositorService.getFocusedScreenName();
        return capable.find(w => w.screenName === focused) ?? capable[0] ?? null;
    }

    /**
     * `claimFocus` is set when the user asked for the prompt (badge click,
     * IPC), which takes the keyboard even when focusMode is "leave".
     */
    function presentPrompt(entry, preferredWidget, claimFocus) {
        root.deferred = false;
        root.promptOpened();

        if (root.placement !== "bar") {
            surface.present(entry.request, claimFocus);
            return;
        }

        const widget = pickBarWidget(preferredWidget);
        if (!widget) {
            waitForHost(entry, claimFocus);
            return;
        }

        root.awaitingHost = false;
        root.barHost = widget;
        widget.beginPrompt(entry.request, claimFocus);
        // Once per request: a prompt moving to another bar is not new.
        if (root.notify && !claimFocus && !entry.notified) {
            entry.notified = true;
            notifyPending(entry.request);
        }
    }

    /**
     * There is nowhere to show a bar prompt. It stays active, so it still
     * times out, and appears as soon as a widget registers.
     */
    function waitForHost(entry, claimFocus) {
        root.awaitingHost = true;
        root.pendingClaim = !!claimFocus;
        if (entry.hostWarned)
            return;
        entry.hostWarned = true;
        warn("no DankBar Pinentry widget in a horizontal bar; the prompt waits for one");
        ToastService.showWarning(I18n.trFor("dankbarPinentry", "Nowhere to show the passphrase prompt"), I18n.trFor("dankbarPinentry", "Add the DankBar Pinentry widget to a horizontal bar, or change its placement. The prompt waits until it times out."));
    }

    /** A widget appeared or became able to host; show a waiting prompt there. */
    function retryHost() {
        if (root.awaitingHost && root.active)
            presentPrompt(root.active, null, root.pendingClaim);
    }

    /** The hosting widget went away or can no longer host; move the prompt. */
    function hostLost(widget) {
        if (root.barHost !== widget)
            return;
        root.barHost = null;
        const claimed = widget.focusClaimed;
        if (widget.hosting)
            widget.endPrompt();
        if (root.active)
            presentPrompt(root.active, null, claimed);
    }

    /** The bar widget, the IPC `open` and the notification lead back to it. */
    function deferPrompt(entry) {
        log("deferring " + entry.request.type + " until opened");
        root.deferred = true;
        if (root.notify)
            notifyPending(entry.request);
    }

    /**
     * A real notification rather than a DMS toast: the toast icon is derived
     * from the severity level and cannot be set.
     */
    function notifyPending(request) {
        const who = describeOwner(request);
        let title = I18n.trFor("dankbarPinentry", "Passphrase required");
        if (request.type === "confirm")
            title = I18n.trFor("dankbarPinentry", "Confirmation required");
        else if (request.type === "message")
            title = I18n.trFor("dankbarPinentry", "Message from gpg-agent");
        // Transient: the bar badge is the durable reminder, so this should not
        // pile up in the notification centre.
        Quickshell.execDetached(["notify-send", "-a", "DankBar Pinentry", "-i", root.notifyIcon, "-u", "normal", "-h", "int:transient:1", title, who ? I18n.trFor("dankbarPinentry", "Requested by %1").arg(who) : ""]);
    }

    function describeOwner(request) {
        if (!root.showOwner || !request.owner)
            return "";
        if (request.owner.command)
            return request.owner.command;
        if (request.owner.pid)
            return I18n.trFor("dankbarPinentry", "pid %1").arg(request.owner.pid);
        return "";
    }

    function openPending(fromWidget) {
        if (!root.active || !root.deferred)
            return;
        presentPrompt(root.active, fromWidget ?? null, true);
    }

    /** Give the keyboard to a prompt already on screen. */
    function claimFocus() {
        if (root.barHost)
            root.barHost.claimFocus();
        else
            surface.claimFocus();
    }

    IpcHandler {
        target: "dankbarPinentry"

        function open(): string {
            if (!root.active)
                return "NO_PROMPT";
            if (root.deferred) {
                root.openPending(null);
                return root.awaitingHost ? "NO_WIDGET" : "OPENED";
            }
            if (root.awaitingHost) {
                root.retryHost();
                return root.awaitingHost ? "NO_WIDGET" : "OPENED";
            }
            root.claimFocus();
            return "FOCUSED";
        }

        function cancel(): string {
            if (!root.active)
                return "NO_PROMPT";
            root.answerCancel();
            return "CANCELLED";
        }

        function status(): string {
            if (!root.active)
                return "idle";
            if (root.deferred)
                return "pending";
            return root.awaitingHost ? "waiting" : "open";
        }
    }

    function registerBarWidget(widget) {
        if (root.barWidgets.indexOf(widget) === -1)
            root.barWidgets = root.barWidgets.concat([widget]);
        retryHost();
    }

    function unregisterBarWidget(widget) {
        root.barWidgets = root.barWidgets.filter(w => w !== widget);
        // Removed from the list first, so the prompt cannot move back to it.
        hostLost(widget);
    }

    /** `repeated`: the prompt had the passphrase typed twice (PROTOCOL.md). */
    function answerPin(pin, repeated) {
        finish({
            v: root.protocolVersion,
            type: "pin",
            id: activeId(),
            pin: pin,
            repeated: !!repeated
        });
    }

    function answerConfirm(outcome) {
        finish({
            v: root.protocolVersion,
            type: "confirm",
            id: activeId(),
            outcome: outcome
        });
    }

    function answerCancel() {
        finish({
            v: root.protocolVersion,
            type: "cancel",
            id: activeId()
        });
    }

    function activeId() {
        return root.active && root.active.request ? root.active.request.id : undefined;
    }

    function finish(reply) {
        const entry = root.active;
        if (!entry) {
            warn("finish() with no active request");
            return;
        }
        // Guards the close handler firing after an explicit answer.
        if (entry.answered)
            return;
        entry.answered = true;

        send(entry.socket, reply);
        closeActive(entry);
    }

    function closeActive(entry) {
        timeoutTimer.stop();
        root.secondsRemaining = -1;

        root.queue = root.queue.filter(e => e !== entry);
        root.active = null;
        root.deferred = false;
        root.awaitingHost = false;
        root.pendingClaim = false;

        const host = root.barHost;
        root.barHost = null;
        if (host)
            host.endPrompt();
        surface.dismiss();
        root.promptClosed();

        // Let the surface tear down before the next one grabs focus.
        Qt.callLater(() => root.activateNext());
    }

    function startTimeout(request) {
        timeoutTimer.stop();
        // A prompt holding an exclusive keyboard grab must never outlive its
        // deadline, or a forgotten dialog locks the session out.
        const secs = request.timeout || 0;
        root.secondsRemaining = secs > 0 ? secs : -1;
        if (secs <= 0)
            return;
        timeoutTimer.interval = secs * 1000;
        timeoutTimer.start();
    }

    Timer {
        id: timeoutTimer

        repeat: false
        onTriggered: {
            root.warn("prompt timed out");
            root.answerCancel();
        }
    }

    Timer {
        interval: 1000
        repeat: true
        running: root.active !== null && root.secondsRemaining > 0
        onTriggered: root.secondsRemaining = Math.max(0, root.secondsRemaining - 1)
    }

    SocketServer {
        id: server

        active: true
        path: root.socketPath

        handler: Socket {
            id: connection

            parser: SplitParser {
                onRead: line => root.handleLine(connection, line)
            }

            onConnectedChanged: {
                if (!connected)
                    root.handleDisconnect(connection);
            }

            onError: err => root.warn("socket error: " + err)
        }
    }

    /**
     * The binary went away. Drop the request rather than leaving a prompt on
     * screen that can never be answered.
     */
    function handleDisconnect(socket) {
        const entry = root.queue.find(e => e.socket === socket);
        if (!entry)
            return;

        entry.answered = true;
        if (root.active === entry) {
            closeActive(entry);
            return;
        }
        root.queue = root.queue.filter(e => e !== entry);
    }

    PinentrySurface {
        id: surface

        daemon: root
        placement: root.placement
        dimBackdrop: root.dimBackdrop
        focusMode: root.focusMode
        showTimeoutRing: root.timeoutRing
        showOwner: root.showOwner

        onSubmitted: (pin, repeated) => root.answerPin(pin, repeated)
        onConfirmed: outcome => root.answerConfirm(outcome)
        onCancelled: root.answerCancel()
    }

    Component.onCompleted: log("listening on " + root.socketPath)

    Component.onDestruction: {
        // Answer outstanding requests so the binary is not left blocked on a
        // socket that is about to disappear.
        for (var i = 0; i < root.queue.length; i++) {
            const entry = root.queue[i];
            if (!entry.answered) {
                entry.answered = true;
                send(entry.socket, {
                    v: root.protocolVersion,
                    type: "cancel",
                    id: entry.request ? entry.request.id : undefined
                });
            }
        }
        server.active = false;
        log("stopped");
    }
}
