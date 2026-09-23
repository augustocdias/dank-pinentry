pragma ComponentBehavior: Bound

import QtQuick
import qs.Common
import qs.Widgets
import qs.Modules.Plugins

/**
 * On NixOS/home-manager the shared plugin settings file is a read-only store
 * symlink, so changes here cannot be saved; set the same keys in
 * `plugins.dankbarPinentry.settings` instead.
 */
PluginSettings {
    id: root

    pluginId: "dankbarPinentry"

    StyledText {
        width: parent.width
        text: I18n.trFor("dankbarPinentry", "DankBar Pinentry")
        font.pixelSize: Theme.fontSizeLarge
        font.weight: Font.Bold
        color: Theme.surfaceText
    }

    StyledText {
        width: parent.width
        text: I18n.trFor("dankbarPinentry", "Requires the %1 binary, set as pinentry-program in gpg-agent.conf.").arg("dank-pinentry")
        font.pixelSize: Theme.fontSizeSmall
        color: Theme.surfaceVariantText
        wrapMode: Text.WordWrap
    }

    SelectionSetting {
        id: placementSetting

        settingKey: "placement"
        label: I18n.trFor("dankbarPinentry", "Placement")
        description: I18n.trFor("dankbarPinentry", "Where the prompt appears on screen. In the bar requires the DankBar Pinentry widget in a horizontal bar.")
        options: [
            {
                label: I18n.trFor("dankbarPinentry", "In the bar"),
                value: "bar"
            },
            {
                label: I18n.trFor("dankbarPinentry", "Centre (dialog)"),
                value: "center"
            },
            {
                label: I18n.trFor("dankbarPinentry", "Top edge"),
                value: "top"
            },
            {
                label: I18n.trFor("dankbarPinentry", "Bottom edge"),
                value: "bottom"
            }
        ]
        defaultValue: "bar"
    }

    ToggleSetting {
        readonly property bool inBar: placementSetting.value === "bar"

        settingKey: "barText"
        label: I18n.trFor("dankbarPinentry", "Show text in the bar")
        description: inBar ? I18n.trFor("dankbarPinentry", "Show the description, errors and the requesting program in the bar. Confirmations always show their question.") : I18n.trFor("dankbarPinentry", "Only applies when the prompt is in the bar.")
        defaultValue: false
        enabled: inBar
        opacity: enabled ? 1 : 0.5
    }

    ToggleSetting {
        readonly property bool inBar: placementSetting.value === "bar"

        settingKey: "dimBackdrop"
        label: I18n.trFor("dankbarPinentry", "Dim the screen")
        description: inBar ? I18n.trFor("dankbarPinentry", "Not available when the prompt is in the bar.") : I18n.trFor("dankbarPinentry", "Darken everything behind the prompt.")
        defaultValue: false
        enabled: !inBar
        opacity: enabled ? 1 : 0.5
    }

    SelectionSetting {
        settingKey: "focusMode"
        label: I18n.trFor("dankbarPinentry", "Keyboard focus")
        description: I18n.trFor("dankbarPinentry", "Escape cancels the prompt in every mode. Holding focus means other windows cannot take the keyboard until the prompt is answered or times out.")
        options: [
            {
                label: I18n.trFor("dankbarPinentry", "Take keyboard focus"),
                value: "take"
            },
            {
                label: I18n.trFor("dankbarPinentry", "Take and hold keyboard focus"),
                value: "hold"
            },
            {
                label: I18n.trFor("dankbarPinentry", "Leave focus where it is (click or IPC to type)"),
                value: "leave"
            }
        ]
        defaultValue: "take"
    }

    ToggleSetting {
        settingKey: "autoOpen"
        label: I18n.trFor("dankbarPinentry", "Open automatically")
        description: I18n.trFor("dankbarPinentry", "Show the prompt as soon as it arrives. With this off it waits in the bar until you click it or run: %1").arg("dms ipc call dankbarPinentry open")
        defaultValue: true
    }

    ToggleSetting {
        settingKey: "notify"
        label: I18n.trFor("dankbarPinentry", "Send a notification")
        description: I18n.trFor("dankbarPinentry", "Post a notification when a passphrase is needed.")
        defaultValue: true
    }

    StringSetting {
        settingKey: "notifyIcon"
        label: I18n.trFor("dankbarPinentry", "Notification icon")
        description: I18n.trFor("dankbarPinentry", "Freedesktop icon name, or an absolute path to an image.")
        placeholder: "dialog-password"
        defaultValue: "dialog-password"
    }

    ToggleSetting {
        settingKey: "showOwner"
        label: I18n.trFor("dankbarPinentry", "Show the requesting program")
        description: I18n.trFor("dankbarPinentry", "Name the process asking for the passphrase. Makes a spoofed prompt easier to spot.")
        defaultValue: true
    }

    ToggleSetting {
        settingKey: "timeoutRing"
        label: I18n.trFor("dankbarPinentry", "Show the countdown")
        description: I18n.trFor("dankbarPinentry", "Display how long is left before the prompt expires.")
        defaultValue: true
    }
}
