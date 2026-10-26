import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt

ColumnLayout {
    id: root
    readonly property var fm: RadioStore.state?.fm ?? ({})
    readonly property bool busy: RadioStore.state?.busy ?? false
    readonly property bool running: fm.running ?? false
    readonly property int frequency: fm.frequency ?? 100000
    readonly property var favorites: fm.favorites ?? []

    spacing: 0

    Item { Layout.fillHeight: true }

    ColumnLayout {
        Layout.alignment: Qt.AlignHCenter
        Layout.minimumHeight: 140
        spacing: 8

        Label {
            Layout.alignment: Qt.AlignHCenter
            text: "FM RADIO"
            color: Theme.onSurfaceVariant
            font.pixelSize: 13
            font.weight: Font.DemiBold
            font.letterSpacing: 4
        }

        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: 8

            Label {
                text: (root.frequency / 1000).toFixed(2)
                color: Theme.onSurface
                font.pixelSize: 88
                font.bold: true
            }
            Label {
                Layout.alignment: Qt.AlignBottom
                Layout.bottomMargin: 10
                text: "MHz"
                color: Theme.onSurfaceVariant
                font.pixelSize: 28
                font.weight: Font.Medium
            }
        }

        Label {
            Layout.alignment: Qt.AlignHCenter
            text: root.fm.stationName || root.fm.stationText || "No station"
            color: Theme.onSurfaceVariant
            font.pixelSize: 16
        }
        Label {
            Layout.alignment: Qt.AlignHCenter
            text: RadioStore.state?.error || (root.running ? "Playing" : "Stopped")
            color: RadioStore.state?.error ? Theme.error : Theme.onSurfaceVariant
            font.pixelSize: 14
        }
    }

    Item { Layout.fillHeight: true }

    RowLayout {
        Layout.alignment: Qt.AlignHCenter
        Layout.bottomMargin: 32
        spacing: 16

        CircleButton {
            iconSource: "../../../assets/icons/mdi/rewind.png"
            enabled: !root.busy && root.running
            onClicked: RadioStore.fm_step(-1, true)
        }
        CircleButton {
            iconSource: "../../../assets/icons/mdi/chevron-left.png"
            enabled: !root.busy && root.running
            onClicked: RadioStore.fm_step(-1, false)
        }
        CircleButton {
            diameter: 76
            primary: true
            iconSource: root.running ? "../../../assets/icons/mdi/stop.png" : "../../../assets/icons/mdi/play.png"
            enabled: !root.busy
            onClicked: RadioStore.fm_toggle()
        }
        CircleButton {
            iconSource: "../../../assets/icons/mdi/chevron-right.png"
            enabled: !root.busy && root.running
            onClicked: RadioStore.fm_step(1, false)
        }
        CircleButton {
            iconSource: "../../../assets/icons/mdi/fast-forward.png"
            enabled: !root.busy && root.running
            onClicked: RadioStore.fm_step(1, true)
        }
    }

    RowLayout {
        Layout.alignment: Qt.AlignHCenter
        spacing: 12

        Repeater {
            model: 5
            delegate: PresetButton {
                id: preset
                required property int index
                readonly property int khz: root.favorites[index] ?? 0

                label: khz > 0 ? (khz / 1000).toFixed(2) : String(index + 1)
                saved: khz > 0
                active: khz > 0 && khz === root.frequency
                enabled: !root.busy
                onTapped: RadioStore.fm_recall_favorite(index)
                onHeld: RadioStore.fm_set_favorite(index)
            }
        }
    }

    PresetHint {
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 12
        Layout.bottomMargin: 40
    }
}
