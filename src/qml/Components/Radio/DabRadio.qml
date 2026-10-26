import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt
import "../Settings"

ColumnLayout {
    id: root
    readonly property var dab: RadioStore.state?.dab ?? ({})
    readonly property bool busy: RadioStore.state?.busy ?? false
    readonly property bool running: dab.running ?? false
    readonly property bool playing: dab.playing ?? false
    readonly property bool scanning: dab.scanning ?? false
    readonly property var stations: dab.stations ?? []
    readonly property var current: dab.current ?? null
    readonly property var favorites: dab.favorites ?? []
    readonly property string error: RadioStore.state?.error ?? ""
    readonly property string statusText: error ? error
        : scanning ? (RadioStore.scanText || "Scanning…")
        : playing ? "Playing"
        : running ? "Ready" : "Stopped"

    spacing: 0

    ColumnLayout {
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 16
        Layout.minimumHeight: 140
        spacing: 8

        Label {
            Layout.alignment: Qt.AlignHCenter
            text: "DAB RADIO"
            color: Theme.onSurfaceVariant
            font.pixelSize: 13
            font.weight: Font.DemiBold
            font.letterSpacing: 4
        }
        Label {
            Layout.alignment: Qt.AlignHCenter
            text: root.current?.label ?? "No station"
            color: Theme.onSurface
            font.pixelSize: 44
            font.bold: true
        }
        Label {
            Layout.alignment: Qt.AlignHCenter
            Layout.minimumHeight: 24
            text: root.dab.dynamicLabel ?? ""
            color: Theme.onSurfaceVariant
            font.pixelSize: 16
        }
        Label {
            Layout.alignment: Qt.AlignHCenter
            text: root.statusText
            color: root.error ? Theme.error : Theme.onSurfaceVariant
            font.pixelSize: 14
        }
    }

    Item {
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.topMargin: 16
        Layout.bottomMargin: 16

        GridView {
            id: grid
            anchors.fill: parent
            visible: root.stations.length > 0
            clip: true
            flow: GridView.FlowTopToBottom
            boundsBehavior: Flickable.StopAtBounds
            cellHeight: height / 2
            cellWidth: cellHeight
            model: root.stations

            ScrollBar.horizontal: ScrollBar {}

            delegate: Item {
                id: cell
                required property var modelData
                readonly property bool active: root.playing && root.current?.id === modelData.id
                    && root.current?.frequency === modelData.frequency

                width: grid.cellWidth
                height: grid.cellHeight

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 6
                    radius: 16
                    color: Theme.surfaceContainerHigh
                    border.width: 2
                    border.color: cell.active ? Theme.primary : "transparent"

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 10
                        spacing: 8

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            radius: 12
                            color: Theme.surfaceContainerHighest

                            SettingsIcon {
                                anchors.centerIn: parent
                                source: "../../../assets/icons/mdi/radio.png"
                                size: 32
                                color: Theme.onSurface
                            }
                        }
                        Label {
                            Layout.fillWidth: true
                            horizontalAlignment: Text.AlignHCenter
                            text: cell.modelData.label
                            color: Theme.onSurface
                            font.weight: Font.DemiBold
                            elide: Text.ElideRight
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !root.busy
                        cursorShape: Qt.PointingHandCursor
                        onClicked: RadioStore.dab_tune(cell.modelData.id, cell.modelData.frequency)
                    }
                }
            }
        }

        Label {
            anchors.centerIn: parent
            visible: root.stations.length === 0 && root.running
            text: "No stations yet - run a scan"
            color: Theme.onSurfaceVariant
            font.pixelSize: 16
        }
    }

    RowLayout {
        Layout.alignment: Qt.AlignHCenter
        Layout.bottomMargin: 16
        spacing: 16

        CircleButton {
            iconSource: "../../../assets/icons/mdi/magnify-scan.png"
            enabled: !root.busy && root.running && !root.scanning
            onClicked: RadioStore.dab_scan()
        }
        CircleButton {
            diameter: 76
            primary: true
            iconSource: root.playing ? "../../../assets/icons/mdi/stop.png" : "../../../assets/icons/mdi/play.png"
            enabled: !root.busy && root.running && !root.scanning && (root.playing || root.current !== null)
            onClicked: RadioStore.dab_toggle_play()
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
                readonly property var favorite: root.favorites[index] ?? null

                label: favorite ? favorite.label : String(index + 1)
                labelSize: 11
                saved: favorite !== null
                active: root.playing && favorite !== null && favorite.id === root.current?.id
                enabled: !root.busy
                onTapped: RadioStore.dab_recall_favorite(index)
                onHeld: RadioStore.dab_set_favorite(index)
            }
        }
    }

    PresetHint {
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 8
        Layout.bottomMargin: 12
    }
}
