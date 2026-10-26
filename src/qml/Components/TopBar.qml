import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import avio_qt

ToolBar {
    id: root
    property string title
    signal navigate(string path)

    readonly property var cards: [slidingCardBluetooth, slidingCardVolume, slidingCardBrightness]

    function toggleCard(card) {
        for (const other of cards) {
            if (other !== card)
                other.open = false
        }
        card.toggle()
    }

    function isInsideCardOrButton(scenePosition) {
        for (const item of [...cards, buttonBluetooth, buttonVolume, buttonBrightness]) {
            if (item.contains(item.mapFromItem(null, scenePosition.x, scenePosition.y)))
                return true
        }
        return false
    }

    // Passive: sees presses anywhere in the window without taking them from the items underneath
    PointHandler {
        parent: root.Window.contentItem
        onActiveChanged: {
            if (active && !root.isInsideCardOrButton(point.scenePosition)) {
                for (const card of root.cards)
                    card.open = false
            }
        }
    }

    // avio: 8px top margin above a 70px row
    implicitHeight: 78
    background: null
    Material.foreground: Theme.onSurface

    RowLayout {
        Layout.fillWidth: true
        anchors.fill: parent
        anchors.topMargin: 8
        anchors.leftMargin: 8
        anchors.rightMargin: 12

        RowLayout {
            Layout.fillHeight: true
            Layout.fillWidth: true
            // avio: ml-2 on each button's wrapper
            spacing: 8

            IconButton {
                id: buttonBluetooth
                icon.source: "../../assets/icons/mdi/bluetooth.png"
                active: slidingCardBluetooth.open
                onClicked: root.toggleCard(slidingCardBluetooth)
            }

            SlidingCard {
                id: slidingCardBluetooth
                target: buttonBluetooth

                RowLayout {
                    Layout.fillWidth: true

                    Label {
                        text: qsTr("Bluetooth")
                        font.pixelSize: 24
                        Layout.fillWidth: true
                    }
                    // No Bluetooth backend yet (avio's switch isn't wired to one either)
                    M3Switch {}
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 12

                    IconButton {
                        enabled: false
                        icon.source: "../../assets/icons/mdi/plus.png"
                    }
                    Label {
                        text: qsTr("To pair a device, open settings.")
                        font.pixelSize: 20
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }
                }

                Button {
                    Layout.fillWidth: true
                    implicitHeight: 56
                    text: qsTr("Open Settings")
                    font.pixelSize: 18
                    Material.foreground: Theme.primary

                    background: Rectangle {
                        radius: height / 2
                        color: Qt.alpha(Theme.primary, 0.12)
                    }
                    HoverHandler {
                        cursorShape: Qt.PointingHandCursor
                    }

                    onClicked: {
                        slidingCardBluetooth.open = false
                        root.navigate("/settings")
                    }
                }
            }


            IconButton {
                id: buttonVolume
                icon.source: "../../assets/icons/mdi/volume-high.png"
                active: slidingCardVolume.open
                onClicked: root.toggleCard(slidingCardVolume)
            }

            SlidingCard {
                id: slidingCardVolume
                target: buttonVolume

                Label {
                    text: qsTr("Audio")
                    font.pixelSize: 20
                }
                Label {
                    text: SettingsStore.defaultSoundDevice
                    opacity: 0.7
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Slider {
                    Layout.fillWidth: true
                    from: 0
                    to: 100
                    stepSize: 1
                    value: SettingsStore.masterVolume
                    onMoved: SettingsStore.masterVolume = value
                }
            }


            IconButton {
                id: buttonBrightness
                icon.source: "../../assets/icons/mdi/brightness-5.png"
                active: slidingCardBrightness.open
                onClicked: root.toggleCard(slidingCardBrightness)
            }

            SlidingCard {
                id: slidingCardBrightness
                target: buttonBrightness

                Label {
                    text: qsTr("Brightness")
                    font.pixelSize: 20
                }
                Slider {
                    Layout.fillWidth: true
                    from: 0
                    to: 100
                    stepSize: 1
                    value: SettingsStore.brightness
                    onMoved: SettingsStore.brightness = value
                }
            }
        }
    }

    Label {
        id: clock
        anchors.horizontalCenter: parent.horizontalCenter
        y: 8 + (70 - height) / 2
        text: Qt.formatTime(new Date(), "HH:mm")
        font.pixelSize: 36
        font.weight: Font.Medium

        Timer {
            interval: 1000
            running: true
            repeat: true
            onTriggered: clock.text = Qt.formatTime(new Date(), "HH:mm")
        }
    }
}
