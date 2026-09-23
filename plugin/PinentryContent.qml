pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import qs.Common
import qs.Widgets

/**
 * The body of a prompt, free of window concerns so the centred dialog and the
 * edge strip can share it. Modelled on DMS's PolkitAuthContent.
 */
FocusScope {
    id: root

    property var request: null
    property bool compact: false
    property bool showTimeoutRing: true
    property bool showOwner: true
    /** -1 when the request has no deadline. */
    property int secondsRemaining: -1
    /** Report the first press, for surfaces that hold back keyboard focus. */
    property bool claimOnPress: false

    /** `repeated`: the user typed it twice and both entries matched. */
    signal submitted(string pin, bool repeated)
    signal confirmed(string outcome)
    signal cancelled
    signal focusClaimRequested

    readonly property string kind: request ? (request.type || "getpin") : "getpin"
    readonly property bool isPinPrompt: kind === "getpin"
    readonly property bool isMessage: kind === "message"
    readonly property bool needsRepeat: isPinPrompt && !!(request && request.repeat)

    readonly property string ownerText: {
        if (!showOwner || !request || !request.owner)
            return "";
        if (request.owner.command)
            return request.owner.command;
        if (request.owner.pid)
            return I18n.trFor("dankbarPinentry", "pid %1").arg(request.owner.pid);
        return "";
    }

    property bool repeatMismatch: false

    readonly property real contentPadding: compact ? Theme.spacingM : Theme.spacingL

    // A FocusScope takes no size from its children.
    implicitWidth: layout.implicitWidth + contentPadding * 2
    implicitHeight: layout.implicitHeight + contentPadding * 2

    function reset() {
        primaryField.text = "";
        repeatField.text = "";
        repeatMismatch = false;
    }

    function focusField() {
        if (root.isPinPrompt)
            primaryField.forceActiveFocus();
        else
            root.forceActiveFocus();
    }

    function submit() {
        if (!root.isPinPrompt) {
            root.confirmed("confirmed");
            return;
        }
        if (root.needsRepeat && primaryField.text !== repeatField.text) {
            root.repeatMismatch = true;
            repeatField.text = "";
            repeatField.forceActiveFocus();
            return;
        }
        root.submitted(primaryField.text, root.needsRepeat);
    }

    Keys.onEscapePressed: root.cancelled()

    Keys.onPressed: event => {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            root.submit();
            event.accepted = true;
        }
    }

    ColumnLayout {
        id: layout

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: root.contentPadding
        spacing: root.compact ? Theme.spacingS : Theme.spacingM

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingM

            DankIcon {
                name: "vpn_key"
                size: Theme.iconSize
                color: Theme.primary
                Layout.alignment: Qt.AlignVCenter
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2

                StyledText {
                    Layout.fillWidth: true
                    text: (root.request && root.request.title) || I18n.trFor("dankbarPinentry", "Passphrase required")
                    font.pixelSize: Theme.fontSizeLarge
                    font.weight: Font.Bold
                    color: Theme.surfaceText
                    elide: Text.ElideRight
                }

                // Naming the requester makes a spoofed prompt harder to pass
                // off as genuine.
                StyledText {
                    Layout.fillWidth: true
                    visible: root.ownerText !== ""
                    text: I18n.trFor("dankbarPinentry", "Requested by %1").arg(root.ownerText)
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.surfaceVariantText
                    elide: Text.ElideMiddle
                }
            }

            Item {
                visible: root.showTimeoutRing && root.secondsRemaining >= 0
                implicitWidth: 32
                implicitHeight: 32
                Layout.alignment: Qt.AlignVCenter

                StyledText {
                    anchors.centerIn: parent
                    text: root.secondsRemaining
                    font.pixelSize: Theme.fontSizeSmall
                    color: root.secondsRemaining <= 10 ? Theme.error : Theme.surfaceVariantText
                }
            }
        }

        StyledText {
            Layout.fillWidth: true
            visible: !!(root.request && root.request.description)
            text: (root.request && root.request.description) || ""
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.surfaceVariantText
            wrapMode: Text.WordWrap
            maximumLineCount: root.compact ? 2 : 6
            elide: Text.ElideRight
        }

        // Error from the previous attempt.
        StyledText {
            Layout.fillWidth: true
            visible: !!(root.request && root.request.error)
            text: (root.request && root.request.error) || ""
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.error
            wrapMode: Text.WordWrap
        }

        DankTextField {
            id: primaryField

            Layout.fillWidth: true
            visible: root.isPinPrompt
            echoMode: TextInput.Password
            showPasswordToggle: true
            backgroundColor: Theme.surfaceHover
            normalBorderColor: Theme.outlineStrong
            focusedBorderColor: Theme.primary
            borderWidth: 1
            focusedBorderWidth: 2
            placeholderText: (root.request && root.request.prompt) || I18n.trFor("dankbarPinentry", "Passphrase")
            onAccepted: root.submit()
        }

        DankTextField {
            id: repeatField

            Layout.fillWidth: true
            visible: root.needsRepeat
            echoMode: TextInput.Password
            showPasswordToggle: true
            backgroundColor: Theme.surfaceHover
            normalBorderColor: root.repeatMismatch ? Theme.error : Theme.outlineStrong
            focusedBorderColor: root.repeatMismatch ? Theme.error : Theme.primary
            borderWidth: 1
            focusedBorderWidth: 2
            placeholderText: (root.request && root.request.repeat) || I18n.trFor("dankbarPinentry", "Repeat")
            onAccepted: root.submit()
        }

        StyledText {
            Layout.fillWidth: true
            visible: root.repeatMismatch
            text: (root.request && root.request.repeatError) || I18n.trFor("dankbarPinentry", "Passphrases do not match")
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.error
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignRight
            spacing: Theme.spacingS

            Item {
                Layout.fillWidth: true
            }

            DankButton {
                visible: !root.isMessage
                text: stripAccelerator((root.request && root.request.cancel) || I18n.trFor("dankbarPinentry", "Cancel"))
                backgroundColor: Theme.surfaceContainerHigh
                textColor: Theme.surfaceText
                onClicked: root.cancelled()
            }

            DankButton {
                visible: !root.isPinPrompt && !root.isMessage && !!(root.request && root.request.notok)
                text: stripAccelerator((root.request && root.request.notok) || I18n.trFor("dankbarPinentry", "No"))
                backgroundColor: Theme.surfaceContainerHigh
                textColor: Theme.surfaceText
                onClicked: root.confirmed("declined")
            }

            DankButton {
                text: stripAccelerator((root.request && root.request.ok) || I18n.trFor("dankbarPinentry", "OK"))
                backgroundColor: Theme.primary
                textColor: Theme.onPrimary
                onClicked: root.submit()
            }
        }
    }

    // On top, but declines the press so it still reaches the field or button
    // underneath.
    MouseArea {
        anchors.fill: parent
        z: 1
        enabled: root.claimOnPress
        onPressed: mouse => {
            root.focusClaimRequested();
            mouse.accepted = false;
        }
    }

    /** gpg-agent marks accelerators with `_`; Qt uses `&`. */
    function stripAccelerator(label) {
        return (label || "").replace(/_/g, "");
    }
}
