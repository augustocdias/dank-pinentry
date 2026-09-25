pragma ComponentBehavior: Bound

import QtQuick
import Quickshell
import Quickshell.Wayland
import qs.Common
import qs.Widgets
import qs.Modals.Common

/**
 * Where the prompt appears: a centred dialog, or a full-width strip against
 * the top or bottom edge. Placement and the visual/focus toggles are
 * orthogonal.
 */
Item {
    id: root

    required property var daemon
    property string placement: "center"
    property bool dimBackdrop: false
    /** "take", "hold" or "leave". */
    property string focusMode: "take"
    property bool showTimeoutRing: true
    property bool showOwner: true

    property var request: null
    property bool active: false
    /** A click, the bar badge or the IPC `open` asked for the keyboard. */
    property bool focusClaimed: false

    signal submitted(string pin, bool repeated)
    signal confirmed(string outcome)
    signal cancelled

    readonly property bool edgePlacement: placement === "top" || placement === "bottom"
    readonly property string effectiveFocus: focusMode === "leave" && focusClaimed ? "take" : focusMode

    function present(request, claimFocus) {
        root.request = request;
        root.focusClaimed = !!claimFocus;
        root.active = true;

        if (root.edgePlacement)
            edgeLoader.active = true;
        else
            centerModal.open();
    }

    function claimFocus() {
        if (!root.active)
            return;
        root.focusClaimed = true;
        Qt.callLater(() => {
            const content = root.edgePlacement ? edgeLoader.item?.contentRef : centerModal.contentLoader?.item;
            if (content)
                content.focusField();
        });
    }

    /**
     * Set while tearing down after an answer, so the close signal is not
     * mistaken for the user dismissing the prompt and turned into a cancel.
     */
    property bool dismissing: false

    function dismiss() {
        root.dismissing = true;
        root.active = false;
        root.request = null;
        root.focusClaimed = false;
        edgeLoader.active = false;
        if (centerModal.shouldBeVisible)
            centerModal.close();
        else
            root.dismissing = false;
    }

    // Its own window because it is independent of placement.
    Loader {
        id: backdropLoader

        active: root.active && root.dimBackdrop && root.edgePlacement

        sourceComponent: PanelWindow {
            color: Qt.rgba(0, 0, 0, 0.45)

            anchors {
                top: true
                bottom: true
                left: true
                right: true
            }

            WlrLayershell.namespace: "dms:plugins:dank-pinentry:backdrop"
            WlrLayershell.layer: WlrLayer.Overlay
            // Purely visual: must never take keystrokes meant for the prompt.
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
            exclusiveZone: 0
        }
    }

    DankModal {
        id: centerModal

        layerNamespace: "dms:plugins:dank-pinentry"
        modalWidth: 460
        modalHeight: contentHeightFor(root.request)
        // Must not be dismissable by a stray click.
        closeOnBackgroundClick: false
        closeOnEscapeKey: true
        allowStacking: true
        keepPopoutsOpen: true
        useOverlayLayer: true
        showBackground: root.dimBackdrop
        backgroundOpacity: root.dimBackdrop ? 0.45 : 0
        // Not OnDemand for "leave": DMS's modal takes a Hyprland focus grab
        // whenever the focus resolves to OnDemand, which would steal focus.
        // None until claimed; the content claims on the first click.
        customKeyboardFocus: {
            if (root.effectiveFocus === "hold")
                return WlrKeyboardFocus.Exclusive;
            if (root.effectiveFocus === "leave")
                return WlrKeyboardFocus.None;
            return null;
        }

        onOpened: Qt.callLater(() => {
            if (contentLoader.item) {
                contentLoader.item.reset();
                contentLoader.item.focusField();
            }
        })

        onDialogClosed: {
            if (root.dismissing) {
                root.dismissing = false;
                return;
            }
            if (root.active)
                root.cancelled();
        }

        content: PinentryContent {
            focus: true
            request: root.request
            compact: false
            showTimeoutRing: root.showTimeoutRing
            showOwner: root.showOwner
            secondsRemaining: root.daemon ? root.daemon.secondsRemaining : -1
            claimOnPress: root.effectiveFocus === "leave"

            onSubmitted: (pin, repeated) => root.submitted(pin, repeated)
            onConfirmed: outcome => root.confirmed(outcome)
            onCancelled: root.cancelled()
            onFocusClaimRequested: root.claimFocus()
        }
    }

    function contentHeightFor(request) {
        if (!request)
            return 220;
        let height = 200;
        if (request.description)
            height += 48;
        if (request.error)
            height += 24;
        if (request.repeat)
            height += 56;
        return Math.min(height, 420);
    }

    Loader {
        id: edgeLoader

        active: false

        sourceComponent: PanelWindow {
            id: strip

            readonly property Item contentRef: stripContent

            color: "transparent"
            // Floored: before the first layout pass the content reports only
            // its padding, and a zero-height layer surface never maps.
            implicitHeight: Math.max(96, Math.ceil(stripContent.implicitHeight))

            anchors {
                left: true
                right: true
                top: root.placement === "top"
                bottom: root.placement === "bottom"
            }

            WlrLayershell.namespace: "dms:plugins:dank-pinentry"
            WlrLayershell.layer: WlrLayer.Overlay
            // OnDemand for "leave" is safe here, unlike the modal: nothing
            // attaches a focus grab to this window unless we ask.
            WlrLayershell.keyboardFocus: {
                if (root.effectiveFocus === "hold")
                    return WlrKeyboardFocus.Exclusive;
                if (root.effectiveFocus === "take")
                    return KeyboardFocus.keyboardFocus(true, null);
                return WlrKeyboardFocus.OnDemand;
            }
            // Overlay rather than reserving space and reflowing every window.
            exclusiveZone: 0

            DankFocusGrab {
                windows: [strip]
                wanted: root.active && root.effectiveFocus === "take" && KeyboardFocus.wantsGrab(true, null)
            }

            Rectangle {
                id: stripBody

                anchors.fill: parent
                color: Theme.withAlpha(Theme.surfaceContainer, Theme.popupTransparency)

                // Border on the exposed edge only, so it reads as attached.
                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: root.placement === "bottom" ? parent.top : undefined
                    anchors.bottom: root.placement === "top" ? parent.bottom : undefined
                    height: 1
                    color: Theme.outlineMedium
                }

                PinentryContent {
                    id: stripContent

                    anchors.fill: parent
                    anchors.leftMargin: Theme.spacingL
                    anchors.rightMargin: Theme.spacingL
                    focus: true
                    request: root.request
                    compact: true
                    showTimeoutRing: root.showTimeoutRing
                    showOwner: root.showOwner
                    secondsRemaining: root.daemon ? root.daemon.secondsRemaining : -1

                    onSubmitted: (pin, repeated) => root.submitted(pin, repeated)
                    onConfirmed: outcome => root.confirmed(outcome)
                    onCancelled: root.cancelled()
                }
            }

            Component.onCompleted: Qt.callLater(() => {
                stripContent.reset();
                stripContent.focusField();
            });
        }
    }
}
