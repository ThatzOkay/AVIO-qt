import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts
import avio_qt
import "../Components/Radio"
import "../Components/Settings"

Item {
    id: root
    property string title: "Media"

    readonly property bool playing: StatusStore.mediaState === "playing"
    readonly property int duration: StatusStore.mediaDurationSeconds

    // The phone only reports the position when something changes, so it's counted up locally in
    // between and snapped back to whatever the phone last said
    property int position: 0
    property int reported: -1

    function syncPosition() {
        const now = StatusStore.mediaPositionSeconds
        if (now === reported)
            return
        reported = now
        position = now
    }

    function clock(seconds) {
        const total = Math.max(0, Math.floor(seconds))
        const minutes = Math.floor(total / 60)
        return minutes + ":" + String(total % 60).padStart(2, "0")
    }

    Component.onCompleted: syncPosition()

    Connections {
        target: StatusStore
        function onMedia_changed() { root.syncPosition() }
    }

    Timer {
        interval: 1000
        repeat: true
        running: root.playing && root.duration > 0
        onTriggered: root.position = Math.min(root.position + 1, root.duration)
    }

    RowLayout {
        anchors.centerIn: parent
        width: Math.min(parent.width - 64, 1000)
        spacing: 48

        // Album art, rounded
        Item {
            id: artFrame
            readonly property int side: Math.min(root.height - 64, root.width * 0.4)
            Layout.preferredWidth: side
            Layout.preferredHeight: side
            Layout.alignment: Qt.AlignVCenter

            Rectangle {
                anchors.fill: parent
                radius: 24
                color: Theme.surfaceContainerHigh
            }
            SettingsIcon {
                anchors.centerIn: parent
                visible: art.status !== Image.Ready
                source: "../../assets/icons/mdi/music-note.png"
                size: 96
                color: Theme.onSurfaceVariant
            }
            Image {
                id: art
                anchors.fill: parent
                source: StatusStore.mediaAlbumArt
                fillMode: Image.PreserveAspectCrop
                visible: false
            }
            Item {
                id: artMask
                anchors.fill: parent
                visible: false
                layer.enabled: true
                Rectangle {
                    anchors.fill: parent
                    radius: 24
                }
            }
            MultiEffect {
                anchors.fill: parent
                source: art
                visible: art.status === Image.Ready
                maskEnabled: true
                maskSource: artMask
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignVCenter
            spacing: 8

            Label {
                Layout.fillWidth: true
                visible: text.length > 0
                text: StatusStore.mediaSource.toUpperCase()
                color: Theme.onSurfaceVariant
                font.pixelSize: 13
                font.weight: Font.DemiBold
                font.letterSpacing: 4
                elide: Text.ElideRight
            }
            Label {
                Layout.fillWidth: true
                text: StatusStore.mediaSong || "Unknown title"
                color: Theme.onSurface
                font.pixelSize: 36
                font.bold: true
                elide: Text.ElideRight
            }
            Label {
                Layout.fillWidth: true
                visible: text.length > 0
                text: StatusStore.mediaArtist
                color: Theme.onSurfaceVariant
                font.pixelSize: 20
                elide: Text.ElideRight
            }
            Label {
                Layout.fillWidth: true
                visible: text.length > 0
                text: StatusStore.mediaAlbum
                color: Theme.onSurfaceVariant
                font.pixelSize: 16
                elide: Text.ElideRight
            }

            // Timeline. Android Auto has no seek command, so this only shows where the song is
            ColumnLayout {
                Layout.fillWidth: true
                Layout.topMargin: 24
                spacing: 6

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 6
                    radius: 3
                    color: Theme.surfaceContainerHighest

                    Rectangle {
                        height: parent.height
                        radius: parent.radius
                        color: Theme.primary
                        width: root.duration > 0
                            ? parent.width * Math.min(1, root.position / root.duration)
                            : 0
                    }
                }
                RowLayout {
                    Layout.fillWidth: true

                    Label {
                        text: root.clock(root.position)
                        color: Theme.onSurfaceVariant
                        font.pixelSize: 13
                    }
                    Item { Layout.fillWidth: true }
                    Label {
                        text: root.duration > 0 ? root.clock(root.duration) : ""
                        color: Theme.onSurfaceVariant
                        font.pixelSize: 13
                    }
                }
            }

            // Shuffle and repeat only mirror the phone, the protocol can't change them
            RowLayout {
                Layout.alignment: Qt.AlignHCenter
                Layout.topMargin: 8
                spacing: 16

                CircleButton {
                    diameter: 52
                    displayOnly: true
                    primary: StatusStore.mediaShuffle
                    iconSource: "../../../assets/icons/mdi/shuffle.png"
                }
                CircleButton {
                    iconSource: "../../../assets/icons/mdi/skip-previous.png"
                    onClicked: StatusStore.media_previous()
                }
                CircleButton {
                    diameter: 76
                    primary: true
                    iconSource: root.playing
                        ? "../../../assets/icons/mdi/pause.png"
                        : "../../../assets/icons/mdi/play.png"
                    onClicked: StatusStore.media_play_pause()
                }
                CircleButton {
                    iconSource: "../../../assets/icons/mdi/skip-next.png"
                    onClicked: StatusStore.media_next()
                }
                CircleButton {
                    diameter: 52
                    displayOnly: true
                    primary: StatusStore.mediaRepeat || StatusStore.mediaRepeatOne
                    iconSource: StatusStore.mediaRepeatOne
                        ? "../../../assets/icons/mdi/repeat-once.png"
                        : "../../../assets/icons/mdi/repeat.png"
                }
            }
        }
    }
}
