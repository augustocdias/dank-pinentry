pragma ComponentBehavior: Bound

import QtQuick
import Quickshell.Wayland
import qs.Common
import qs.Widgets
import qs.Modules.Plugins

/**
 * A badge while a request waits to be opened and, with the `bar` placement,
 * the prompt itself: a passphrase field, or a question with buttons.
 *
 * DMS gives a plain bar no keyboard focus and has no plugin API for it, so
 * while hosting a prompt this widget overrides its own bar window's
 * `WlrLayershell.keyboardFocus`, using the same helpers DMS uses for its
 * modals.
 */
PluginComponent {
    id: root

    layerNamespacePlugin: "dank-pinentry"

    // Re-evaluates when PluginService reassigns the map, e.g. on plugin reload.
    readonly property var daemon: (pluginService && pluginId) ? (pluginService.pluginDaemonInstances[pluginId] ?? null) : null
    property var registeredWith: null

    readonly property var hostWindow: surfaceContext?.hostWindow ?? null
    readonly property string screenName: parentScreen?.name ?? ""
    readonly property string barId: surfaceContext?.configId ?? barConfig?.id ?? ""
    // Vertical bars are too narrow, and island bars manage their own focus.
    readonly property bool canHostPrompt: !!hostWindow && !isVertical && (surfaceContext?.kind ?? "bar") === "bar" && barConfig?.island !== true && !(blurBarWindow?.isIsland ?? false)

    property var request: null
    readonly property bool hosting: request !== null
    readonly property string kind: request?.type ?? "getpin"
    readonly property bool isPin: kind === "getpin"
    readonly property bool badgeVisible: !hosting && !!daemon && daemon.deferred
    readonly property int waitingCount: daemon ? daemon.queue.length : 0

    /** A click, the badge or the IPC `open` asked for the keyboard. */
    property bool focusClaimed: false
    readonly property string focusMode: daemon?.focusMode ?? "take"
    readonly property string effectiveFocus: focusMode === "leave" && focusClaimed ? "take" : focusMode
    readonly property bool focusExpected: effectiveFocus !== "leave"
    // OnDemand for "leave": a click on the prompt focuses the bar. Safe here
    // because DMS attaches no focus grab to a non-island bar.
    readonly property var layerFocus: {
        if (effectiveFocus === "hold")
            return WlrKeyboardFocus.Exclusive;
        if (effectiveFocus === "take")
            return KeyboardFocus.keyboardFocus(true, null);
        return WlrKeyboardFocus.OnDemand;
    }

    readonly property real inputHeight: Math.round(Math.max(textSize + 6, widgetThickness * 0.75))
    readonly property int timeoutSecs: request?.timeout ?? 0
    readonly property real remainingFraction: timeoutSecs > 0 && daemon ? Math.max(0, daemon.secondsRemaining) / timeoutSecs : 0

    readonly property bool showText: daemon?.barText ?? false
    readonly property string fullDescription: (request?.description || request?.title || "").trim()
    readonly property string descriptionLine: fullDescription.replace(/\s*\n\s*/g, " ")
    readonly property string ownerLine: daemon && request ? daemon.describeOwner(request) : ""
    // A confirmation is meaningless without its question, so that always shows.
    readonly property bool showErrorText: showText && !!request?.error
    readonly property bool showDescription: (showText || !isPin) && descriptionLine !== ""
    readonly property bool showOwnerText: showText && ownerLine !== ""
    readonly property real maxTextWidth: Math.round((parentScreen?.width ?? 1920) * 0.3)
    readonly property string tooltipText: {
        const parts = [];
        if (showErrorText)
            parts.push(request.error);
        if (showDescription)
            parts.push(fullDescription);
        if (showOwnerText)
            parts.push(I18n.trFor("dankbarPinentry", "Requested by %1").arg(ownerLine));
        return parts.join("\n");
    }

    /** Cancel, the optional "no" and OK, in gpg-agent's labels; OK is last. */
    readonly property var buttons: {
        if (!request || isPin)
            return [];
        const ok = {
            id: "ok",
            label: stripAccelerator(request.ok) || I18n.trFor("dankbarPinentry", "OK"),
            filled: true
        };
        if (kind === "message")
            return [ok];
        const list = [
            {
                id: "cancel",
                label: stripAccelerator(request.cancel) || I18n.trFor("dankbarPinentry", "Cancel"),
                filled: false
            }
        ];
        if (request.notok)
            list.push({
                id: "notok",
                label: stripAccelerator(request.notok),
                filled: false
            });
        list.push(ok);
        return list;
    }
    property int buttonIndex: 0

    property Item inputItem: null
    property Item buttonsItem: null
    readonly property Item focusTarget: isPin ? inputItem : buttonsItem
    property bool typedSinceError: false
    property bool revealedByUs: false
    property int focusAttempts: 0

    signal shakeRequested

    onShouldShowChanged: setVisibilityOverride(shouldShow)
    readonly property bool shouldShow: hosting || badgeVisible

    onDaemonChanged: syncRegistration()

    onCanHostPromptChanged: {
        if (!registeredWith)
            return;
        if (canHostPrompt)
            registeredWith.retryHost();
        else if (hosting)
            registeredWith.hostLost(root);
    }

    Component.onCompleted: {
        setVisibilityOverride(shouldShow);
        syncRegistration();
    }

    Component.onDestruction: {
        hideTooltip();
        holdBarRevealed(false);
        if (registeredWith)
            registeredWith.unregisterBarWidget(root);
    }

    function syncRegistration() {
        if (registeredWith === daemon)
            return;
        // Otherwise a prompt from a destroyed daemon would hold the bar's
        // keyboard focus indefinitely.
        if (hosting)
            endPrompt();
        // A destroyed daemon reads as null here, so there is nothing to undo.
        if (registeredWith)
            registeredWith.unregisterBarWidget(root);
        registeredWith = daemon;
        if (daemon)
            daemon.registerBarWidget(root);
    }

    function beginPrompt(req, claimFocus) {
        typedSinceError = false;
        focusClaimed = !!claimFocus;
        if (inputItem)
            inputItem.text = "";
        request = req;
        buttonIndex = Math.max(0, buttons.length - 1);
        holdBarRevealed(true);

        focusAttempts = 0;
        focusTimer.restart();

        if (isPin && req.error)
            Qt.callLater(() => root.shakeRequested());
    }

    function claimFocus() {
        if (!hosting)
            return;
        focusClaimed = true;
        focusAttempts = 0;
        focusTimer.restart();
    }

    function endPrompt() {
        focusTimer.stop();
        hideTooltip();
        if (inputItem)
            inputItem.text = "";
        request = null;
        focusClaimed = false;
        holdBarRevealed(false);
    }

    function submit() {
        if (!hosting || !isPin || !daemon || !inputItem)
            return;
        const pin = inputItem.text;
        inputItem.text = "";
        // One field: gpg-agent does its own repeat check (PROTOCOL.md).
        daemon.answerPin(pin, false);
    }

    function cancel() {
        if (!hosting || !daemon)
            return;
        if (inputItem)
            inputItem.text = "";
        daemon.answerCancel();
    }

    function chooseButton(index) {
        const button = buttons[index];
        if (!hosting || !daemon || !button)
            return;
        if (button.id === "ok")
            daemon.answerConfirm("confirmed");
        else if (button.id === "notok")
            daemon.answerConfirm("declined");
        else
            daemon.answerCancel();
    }

    function moveButton(delta) {
        const count = buttons.length;
        if (count > 0)
            buttonIndex = (buttonIndex + delta + count) % count;
    }

    /** gpg-agent marks accelerators with `_`; meaningless in the bar. */
    function stripAccelerator(label) {
        return (label || "").replace(/_/g, "");
    }

    /** An auto-hidden bar would slide away mid-typing. Runtime state, not a settings write. */
    function holdBarRevealed(hold) {
        if (!barId || typeof SettingsData.setBarIpcReveal !== "function")
            return;
        if (hold && !SettingsData.isBarIpcRevealed(barId)) {
            SettingsData.setBarIpcReveal(barId, true);
            revealedByUs = true;
        } else if (!hold && revealedByUs) {
            SettingsData.setBarIpcReveal(barId, false);
            revealedByUs = false;
        }
    }

    function showTooltip(anchorItem) {
        if (!tooltipText || !parentScreen)
            return;
        tooltipLoader.active = true;
        if (!tooltipLoader.item)
            return;
        const pos = anchorItem.mapToItem(null, anchorItem.width / 2, 0);
        let y = barThickness + barSpacing + Theme.spacingXS;
        if (axis?.edge === "bottom") {
            const lines = tooltipText.split("\n").length;
            const height = lines * Theme.fontSizeSmall * 1.5 + Theme.spacingS * 2;
            y = parentScreen.height - barThickness - barSpacing - Theme.spacingXS - height;
        }
        tooltipLoader.item.show(tooltipText, pos.x, y, parentScreen, false, false);
    }

    function hideTooltip() {
        if (tooltipLoader.item)
            tooltipLoader.item.hide();
        tooltipLoader.active = false;
    }

    Loader {
        id: tooltipLoader

        active: false
        sourceComponent: DankTooltip {}
    }

    Binding {
        // Kept stable rather than tied to `hosting`, so the restore on release
        // still has a target.
        target: root.hostWindow ? root.hostWindow.WlrLayershell : null
        property: "keyboardFocus"
        value: root.layerFocus
        when: root.hosting && !!root.hostWindow
        restoreMode: Binding.RestoreBindingOrValue
    }

    DankFocusGrab {
        windows: root.hostWindow ? [root.hostWindow] : []
        wanted: root.hosting && root.effectiveFocus === "take" && KeyboardFocus.wantsGrab(true, null)
    }

    Timer {
        id: focusTimer

        interval: 100
        repeat: true
        onTriggered: {
            const target = root.focusTarget;
            // activeFocus only turns true once the compositor has actually
            // given the window the keyboard.
            if (target && target.activeFocus) {
                stop();
                return;
            }
            if (target)
                target.forceActiveFocus();
            if (!root.focusExpected) {
                stop();
                return;
            }
            root.focusAttempts++;
            if (root.focusAttempts >= 10) {
                stop();
                console.warn("[dankbarPinentry] the bar did not receive keyboard focus; click the prompt to type");
            }
        }
    }

    pillClickAction: function () {
        if (root.hosting)
            root.claimFocus();
        else if (root.daemon)
            root.daemon.openPending(root);
    }

    horizontalBarPill: Component {
        Item {
            implicitWidth: root.hosting ? controls.implicitWidth : badge.implicitWidth
            implicitHeight: root.hosting ? controls.implicitHeight : badge.implicitHeight

            Row {
                id: badge

                anchors.centerIn: parent
                visible: !root.hosting
                spacing: Theme.spacingXS

                DankIcon {
                    name: "vpn_key"
                    size: root.iconSize
                    color: Theme.warning
                    anchors.verticalCenter: parent.verticalCenter
                }

                StyledText {
                    visible: root.waitingCount > 1
                    text: root.waitingCount
                    font.pixelSize: root.textSize
                    color: Theme.surfaceText
                    anchors.verticalCenter: parent.verticalCenter
                }
            }

            Row {
                id: controls

                anchors.centerIn: parent
                visible: root.hosting
                spacing: Theme.spacingS

                Row {
                    id: texts

                    visible: root.showErrorText || root.showDescription || root.showOwnerText
                    spacing: Theme.spacingS
                    anchors.verticalCenter: parent.verticalCenter

                    StyledText {
                        visible: root.showErrorText
                        width: Math.min(implicitWidth, root.maxTextWidth / 3)
                        text: root.request?.error ?? ""
                        elide: Text.ElideRight
                        font.pixelSize: root.textSize
                        color: Theme.error
                        anchors.verticalCenter: parent.verticalCenter
                    }

                    StyledText {
                        visible: root.showDescription
                        width: Math.min(implicitWidth, root.maxTextWidth)
                        text: root.descriptionLine
                        // Middle, so a question at the end stays readable.
                        elide: Text.ElideMiddle
                        font.pixelSize: root.textSize
                        color: Theme.surfaceText
                        anchors.verticalCenter: parent.verticalCenter
                    }

                    StyledText {
                        visible: root.showOwnerText
                        width: Math.min(implicitWidth, root.maxTextWidth / 3)
                        text: root.ownerLine
                        elide: Text.ElideMiddle
                        font.pixelSize: root.textSize
                        color: Theme.surfaceVariantText
                        anchors.verticalCenter: parent.verticalCenter
                    }

                    HoverHandler {
                        onHoveredChanged: {
                            if (hovered)
                                root.showTooltip(texts);
                            else
                                root.hideTooltip();
                        }
                    }
                }

                Rectangle {
                    id: field

                    readonly property bool errorState: !!root.request?.error && !root.typedSinceError

                    visible: root.isPin
                    anchors.verticalCenter: parent.verticalCenter
                    height: root.inputHeight
                    width: Math.round(height * 8)
                    radius: Math.min(Theme.cornerRadius, height / 2)
                    color: Theme.surfaceContainerHighest
                    border.width: 1
                    border.color: errorState ? Theme.error : (input.activeFocus ? Theme.primary : Theme.outlineStrong)

                    transform: Translate {
                        id: shakeOffset
                    }

                    TextInput {
                        id: input

                        anchors.fill: parent
                        anchors.leftMargin: Math.round(parent.height * 0.4)
                        anchors.rightMargin: anchors.leftMargin
                        verticalAlignment: TextInput.AlignVCenter
                        enabled: root.hosting && root.isPin
                        echoMode: TextInput.Password
                        // Qt already implies these for password echo; explicit so a
                        // change of echo mode cannot silently drop them.
                        inputMethodHints: Qt.ImhSensitiveData | Qt.ImhNoPredictiveText | Qt.ImhHiddenText | Qt.ImhNoAutoUppercase
                        font.pixelSize: root.textSize
                        color: Theme.surfaceText
                        selectionColor: Theme.primary
                        clip: true

                        onTextEdited: root.typedSinceError = true
                        onAccepted: root.submit()
                        Keys.onEscapePressed: event => {
                            event.accepted = true;
                            root.cancel();
                        }

                        Component.onCompleted: root.inputItem = input
                        Component.onDestruction: {
                            if (root.inputItem === input)
                                root.inputItem = null;
                        }
                    }
                }

                Row {
                    id: buttonRow

                    visible: root.hosting && !root.isPin
                    spacing: Theme.spacingXS
                    anchors.verticalCenter: parent.verticalCenter

                    Keys.onPressed: event => {
                        switch (event.key) {
                        case Qt.Key_Escape:
                            root.cancel();
                            break;
                        case Qt.Key_Return:
                        case Qt.Key_Enter:
                        case Qt.Key_Space:
                            root.chooseButton(root.buttonIndex);
                            break;
                        case Qt.Key_Left:
                        case Qt.Key_Backtab:
                            root.moveButton(-1);
                            break;
                        case Qt.Key_Right:
                        case Qt.Key_Tab:
                            root.moveButton(1);
                            break;
                        default:
                            return;
                        }
                        event.accepted = true;
                    }

                    Repeater {
                        model: root.buttons

                        delegate: Rectangle {
                            id: button

                            required property var modelData
                            required property int index

                            // Only shown while the row holds the keyboard, so
                            // it never suggests focus the bar does not have.
                            readonly property bool selected: buttonRow.activeFocus && root.buttonIndex === index
                            readonly property bool filled: modelData.filled

                            anchors.verticalCenter: parent.verticalCenter
                            height: root.inputHeight
                            width: Math.max(Math.round(height * 2), label.implicitWidth + Math.round(height * 0.9))
                            radius: Math.min(Theme.cornerRadius, height / 2)
                            color: {
                                if (filled)
                                    return buttonMouse.containsMouse ? Qt.lighter(Theme.primary, 1.1) : Theme.primary;
                                return buttonMouse.containsMouse ? Theme.surfaceContainerHigh : Theme.surfaceContainerHighest;
                            }
                            border.width: selected ? 2 : 1
                            border.color: {
                                if (selected)
                                    return filled ? Theme.surfaceText : Theme.primary;
                                return filled ? Theme.primary : Theme.outlineStrong;
                            }

                            StyledText {
                                id: label

                                anchors.centerIn: parent
                                text: button.modelData.label
                                font.pixelSize: root.textSize
                                color: button.filled ? Theme.onPrimary : Theme.surfaceText
                            }

                            MouseArea {
                                id: buttonMouse

                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: root.chooseButton(button.index)
                            }
                        }
                    }

                    Component.onCompleted: root.buttonsItem = buttonRow
                    Component.onDestruction: {
                        if (root.buttonsItem === buttonRow)
                            root.buttonsItem = null;
                    }
                }
            }

            // Below the whole prompt, outside the layout so the controls stay
            // centred in the bar.
            Rectangle {
                visible: root.hosting && (root.daemon?.timeoutRing ?? true) && root.timeoutSecs > 0
                anchors.top: controls.bottom
                anchors.topMargin: 1
                x: controls.x
                height: 2
                radius: 1
                width: controls.width * root.remainingFraction
                color: root.remainingFraction <= 0.2 ? Theme.error : Theme.primary

                Behavior on width {
                    NumberAnimation {
                        duration: 1000
                    }
                }
            }

            SequentialAnimation {
                id: shake

                NumberAnimation {
                    target: shakeOffset
                    property: "x"
                    to: -6
                    duration: 50
                }
                NumberAnimation {
                    target: shakeOffset
                    property: "x"
                    to: 6
                    duration: 80
                }
                NumberAnimation {
                    target: shakeOffset
                    property: "x"
                    to: -4
                    duration: 70
                }
                NumberAnimation {
                    target: shakeOffset
                    property: "x"
                    to: 0
                    duration: 60
                }
            }

            Connections {
                target: root

                function onShakeRequested() {
                    shake.restart();
                }
            }
        }
    }

    verticalBarPill: Component {
        Column {
            spacing: Theme.spacingXS

            DankIcon {
                name: "vpn_key"
                size: root.iconSize
                color: Theme.warning
                anchors.horizontalCenter: parent.horizontalCenter
            }

            StyledText {
                visible: root.waitingCount > 1
                text: root.waitingCount
                font.pixelSize: root.textSize
                color: Theme.surfaceText
                anchors.horizontalCenter: parent.horizontalCenter
            }
        }
    }
}
