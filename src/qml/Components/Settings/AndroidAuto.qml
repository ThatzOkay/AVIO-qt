import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt
import ".."

ColumnLayout {
    readonly property bool connected: StatusStore.aaStatus === "connected" || StatusStore.aaStatus === "host-ui"

    spacing: 12

    Label {
        text: "Android Auto"
        color: Theme.onSurface
        font.pixelSize: 24
        Layout.bottomMargin: 4
    }

    SettingsCard {
        SettingsRow {
            iconSource: "../../../assets/icons/mdi/connection.png"
            title: "Status"
            subtitle: "Connected to Android Auto"

            trailing: Rectangle {
                implicitWidth: statusLabel.implicitWidth + 32
                implicitHeight: 32
                radius: 16
                color: "transparent"
                border.width: 1
                border.color: Theme.outline

                Label {
                    id: statusLabel
                    anchors.centerIn: parent
                    text: connected ? "Connected" : "Disconnected"
                    color: Theme.primary
                    font.pixelSize: 14
                }
            }
        }
        SettingsDivider {}
        SettingsRow {
            iconSource: "../../../assets/icons/mdi/cellphone.png"
            title: "Device"
            subtitle: StatusStore.aaDeviceName || "Not connected"
        }
    }

    SettingsCard {
        SettingsSubheader { text: "Video quality" }

        SettingsRow {
            iconSource: "../../../assets/icons/mdi/television.png"
            title: "Resolution"

            trailing: SettingsChips {
                options: [
                    { label: "Auto", value: "auto" },
                    { label: "480p", value: "480p" },
                    { label: "720p", value: "720p" },
                    { label: "1080p", value: "1080p" }
                ]
                current: SettingsStore.aoResolution
                onSelected: value => SettingsStore.aoResolution = value
            }
        }
        SettingsDivider {}
        SettingsRow {
            iconSource: "../../../assets/icons/mdi/artboard.png"
            title: "Frame rate"

            trailing: SettingsChips {
                options: [
                    { label: "30 fps", value: "30" },
                    { label: "60 fps", value: "60" }
                ]
                current: SettingsStore.aoFramerate
                onSelected: value => SettingsStore.aoFramerate = value
            }
        }
        SettingsDivider {}
        SettingsRow {
            iconSource: "../../../assets/icons/mdi/image-size-select-large.png"
            title: "Dpi"
            subtitle: SettingsStore.aoDpiAuto ? "Picked from the video resolution and the screen size when a phone connects" : ""

            trailing: RowLayout {
                spacing: 16

                Label {
                    visible: !SettingsStore.aoDpiAuto
                    text: Math.round(SettingsStore.aoDpi)
                    color: Theme.primary
                    font.pixelSize: 14
                }
                Label {
                    text: "Auto"
                    color: Theme.onSurfaceVariant
                    font.pixelSize: 14
                }
                M3Switch {
                    checked: SettingsStore.aoDpiAuto
                    onToggled: SettingsStore.aoDpiAuto = checked
                }
            }
        }
        SettingsSlider {
            visible: !SettingsStore.aoDpiAuto
            Layout.leftMargin: 28
            Layout.rightMargin: 28
            Layout.bottomMargin: 8
            startIcon: "../../../assets/icons/mdi/format-font-size-increase.png"
            endIcon: "../../../assets/icons/mdi/format-font-size-decrease.png"
            from: 96
            to: 213
            stepSize: 1
            value: SettingsStore.aoDpi
            onMoved: value => SettingsStore.aoDpi = value
        }
    }

    SettingsCard {
        SettingsSubheader { text: "Screen geometry" }

        SettingsRow {
            iconSource: "../../../assets/icons/mdi/arrow-expand-all.png"
            title: "View area insets"
            subtitle: "Extra space reserved around the projected video, on top of the automatic aspect-ratio letterboxing"
        }
        SettingsInsets {
            Layout.leftMargin: 28
            Layout.rightMargin: 28
            Layout.bottomMargin: 8
            insetTop: SettingsStore.aoViewAreaTop
            insetBottom: SettingsStore.aoViewAreaBottom
            insetLeft: SettingsStore.aoViewAreaLeft
            insetRight: SettingsStore.aoViewAreaRight
            onEdited: (side, value) => {
                if (side === "top") SettingsStore.aoViewAreaTop = value
                else if (side === "bottom") SettingsStore.aoViewAreaBottom = value
                else if (side === "left") SettingsStore.aoViewAreaLeft = value
                else SettingsStore.aoViewAreaRight = value
            }
        }
        SettingsDivider {}
        SettingsRow {
            iconSource: "../../../assets/icons/mdi/crop-free.png"
            title: "Safe area insets"
            subtitle: "Content insets the phone keeps clear of overlays (e.g. a dashboard cutout), without affecting the letterbox crop"
        }
        SettingsInsets {
            Layout.leftMargin: 28
            Layout.rightMargin: 28
            Layout.bottomMargin: 8
            insetTop: SettingsStore.aoSafeAreaTop
            insetBottom: SettingsStore.aoSafeAreaBottom
            insetLeft: SettingsStore.aoSafeAreaLeft
            insetRight: SettingsStore.aoSafeAreaRight
            onEdited: (side, value) => {
                if (side === "top") SettingsStore.aoSafeAreaTop = value
                else if (side === "bottom") SettingsStore.aoSafeAreaBottom = value
                else if (side === "left") SettingsStore.aoSafeAreaLeft = value
                else SettingsStore.aoSafeAreaRight = value
            }
        }
    }
}
