import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt

ColumnLayout {
    spacing: 12

    Label {
        text: "Sound"
        color: Theme.onSurface
        font.pixelSize: 24
        Layout.bottomMargin: 4
    }

    SettingsCard {
        SettingsSubheader { text: "Volume" }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 28
            Layout.rightMargin: 28
            Layout.bottomMargin: 8
            spacing: 4

            RowLayout {
                spacing: 8
                SettingsIcon {
                    source: "../../../assets/icons/mdi/volume-high.png"
                    size: 18
                }
                Label {
                    text: "Master"
                    color: Theme.onSurfaceVariant
                    font.pixelSize: 14
                }
            }

            SettingsSlider {
                from: 0
                to: 100
                value: SettingsStore.masterVolume
                onMoved: value => SettingsStore.masterVolume = Math.round(value)
            }
        }
    }

    SettingsCard {
        SettingsSubheader { text: "Default Sound Device" }

        Repeater {
            model: SettingsStore.audioDevices
            delegate: ColumnLayout {
                id: output
                required property int index
                required property string modelData
                readonly property bool isDefault: modelData === SettingsStore.defaultSoundDevice

                Layout.fillWidth: true
                spacing: 0

                SettingsRow {
                    iconSource: "../../../assets/icons/mdi/speaker.png"
                    title: output.modelData
                    subtitle: output.isDefault ? "Default" : ""
                    selected: output.isDefault
                    interactive: true
                    onClicked: SettingsStore.defaultSoundDevice = output.modelData
                }
                SettingsDivider {
                    visible: output.index < SettingsStore.audioDevices.length - 1
                }
            }
        }
    }

    SettingsCard {
        SettingsSubheader { text: "Default Input Device" }

        Repeater {
            model: SettingsStore.inputDevices
            delegate: ColumnLayout {
                id: input
                required property int index
                required property string modelData
                readonly property bool isDefault: modelData === SettingsStore.defaultInputDevice

                Layout.fillWidth: true
                spacing: 0

                SettingsRow {
                    iconSource: "../../../assets/icons/mdi/microphone.png"
                    title: input.modelData
                    subtitle: input.isDefault ? "Default" : ""
                    selected: input.isDefault
                    interactive: true
                    onClicked: SettingsStore.defaultInputDevice = input.modelData
                }
                SettingsDivider {
                    visible: input.index < SettingsStore.inputDevices.length - 1
                }
            }
        }
    }
}
