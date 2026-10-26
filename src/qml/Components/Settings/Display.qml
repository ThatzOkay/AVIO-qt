import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt
import ".."

ColumnLayout {
    spacing: 12

    Label {
        text: "Display"
        color: Theme.onSurface
        font.pixelSize: 24
        Layout.bottomMargin: 4
    }

    SettingsCard {
        SettingsSubheader { text: "Brightness" }

        SettingsSlider {
            Layout.leftMargin: 28
            Layout.rightMargin: 28
            Layout.bottomMargin: 8
            startIcon: "../../../assets/icons/mdi/brightness-5.png"
            endIcon: "../../../assets/icons/mdi/brightness-7.png"
            from: 0
            to: 100
            value: SettingsStore.brightness
            onMoved: value => SettingsStore.brightness = Math.round(value)
        }
    }

    SettingsCard {
        SettingsSubheader { text: "Theme" }

        SettingsRow {
            iconSource: "../../../assets/icons/mdi/weather-night.png"
            title: "Dark"
            subtitle: Theme.dark ? "Current" : ""
            selected: Theme.dark
            interactive: true
            onClicked: Theme.dark = true
        }
        SettingsDivider {}
        SettingsRow {
            iconSource: "../../../assets/icons/mdi/white-balance-sunny.png"
            title: "Light"
            subtitle: !Theme.dark ? "Current" : ""
            selected: !Theme.dark
            interactive: true
            onClicked: Theme.dark = false
        }
        SettingsDivider {}
        SettingsRow {
            id: baseColorRow
            iconSource: "../../../assets/icons/mdi/palette.png"
            title: "Base Color"
            subtitle: "Change the base color of the app"
            interactive: true
            onClicked: colorPopup.opened ? colorPopup.close() : colorPopup.open()

            trailing: RowLayout {
                spacing: 8
                Label {
                    text: Theme.seed.toUpperCase()
                    color: Theme.onSurfaceVariant
                    font.family: "monospace"
                    font.pixelSize: 14
                }
                Rectangle {
                    implicitWidth: 24
                    implicitHeight: 24
                    radius: 12
                    color: Theme.seed
                    border.width: 2
                    border.color: Theme.outlineVariant
                }
            }

            Popup {
                id: colorPopup
                y: baseColorRow.height
                x: baseColorRow.width - width
                width: 280
                padding: 16
                enter: null
                exit: null
                background: Rectangle {
                    radius: 12
                    color: Theme.surfaceContainer
                }

                contentItem: SettingsColorPicker {
                    color: Theme.seed
                    onPicked: value => Theme.seed = value.toString()
                }
            }
        }
    }

    SettingsCard {
        SettingsSubheader { text: "Kiosk & output" }

        SettingsRow {
            iconSource: "../../../assets/icons/mdi/fullscreen.png"
            title: "Kiosk mode"
            subtitle: "Always start fullscreen and stay there. Takes effect on next launch."

            trailing: M3Switch {
                checked: SettingsStore.kiosk
                onToggled: SettingsStore.kiosk = checked
            }
        }
        SettingsDivider {}
        SettingsRow {
            iconSource: "../../../assets/icons/mdi/monitor-screenshot.png"
            title: "Display mode"
            subtitle: "Physical output resolution/refresh rate, applied when kiosk mode starts. Takes effect on next launch."

            trailing: ComboBox {
                implicitWidth: 180
                model: ["Panel default"].concat(SettingsStore.displayModes)
                currentIndex: Math.max(0, SettingsStore.displayModes.indexOf(SettingsStore.displayMode) + 1)
                onActivated: index => SettingsStore.displayMode = index === 0 ? "" : SettingsStore.displayModes[index - 1]
            }
        }
    }
}
