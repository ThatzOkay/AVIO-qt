import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import QtQuick.Controls.Material
import avio_qt

Item {
    id: root
    property string title: "Settings"

    readonly property var categories: [
        {
            id: "connected-devices",
            label: "Connected Devices",
            icon: "../../assets/icons/mdi/cellphone.png"
        },
        {
            id: "sound",
            label: "Sound",
            icon: "../../assets/icons/mdi/volume-high.png"
        },
        {
            id: "display",
            label: "Display",
            icon: "../../assets/icons/mdi/monitor.png"
        },
        {
            id: "android-auto",
            label: "Android Auto",
            icon: "../../assets/icons/mdi/car.png"
        },
        {
            id: "radio",
            label: "Radio",
            icon: "../../assets/icons/mdi/radio.png"
        },
        {
            id: "updates",
            label: "Updates",
            icon: "../../assets/icons/mdi/update.png"
        },
        {
            id: "about",
            label: "About",
            icon: "../../assets/icons/mdi/information-outline.png"
        }
    ]

    readonly property var categoryComponents: {
        "connected-devices": "../Components/Settings/ConnectedDevices.qml",
        "sound": "../Components/Settings/Sound.qml",
        "display": "../Components/Settings/Display.qml",
        "android-auto": "../Components/Settings/AndroidAuto.qml",
        "radio": "../Components/Settings/DabStations.qml",
        "updates": "../Components/Settings/Updates.qml",
        "about": "../Components/Settings/About.qml"
    }

    property string current: "connected-devices"

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // Nav rail
        ColumnLayout {
            Layout.fillWidth: false
            Layout.minimumWidth: 280
            Layout.preferredWidth: 280
            Layout.maximumWidth: 280
            Layout.fillHeight: true
            spacing: 0

            Label {
                Layout.fillWidth: true
                text: root.title
                color: Theme.onSurface
                font.pixelSize: 22
                font.weight: Font.Medium
                topPadding: 16
                bottomPadding: 16
                leftPadding: 24
                background: Rectangle {
                    topLeftRadius: 28
                    color: Theme.surfaceContainerLow
                }
            }

            ListView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                topMargin: 8
                bottomMargin: 8
                leftMargin: 8
                rightMargin: 8
                spacing: 4
                clip: true
                model: root.categories

                delegate: ItemDelegate {
                    id: item
                    required property var modelData
                    readonly property bool active: root.current === modelData.id

                    width: ListView.view.width - ListView.view.leftMargin - ListView.view.rightMargin
                    height: 48
                    leftPadding: 16
                    rightPadding: 16
                    spacing: 16
                    text: modelData.label
                    font.pixelSize: 16
                    icon.source: modelData.icon
                    icon.width: 24
                    icon.height: 24
                    icon.color: active ? Theme.primary : Theme.onSurfaceVariant
                    Material.foreground: active ? Theme.primary : Theme.onSurface
                    onClicked: root.current = modelData.id

                    background: Rectangle {
                        radius: 28
                        color: item.active ? Qt.alpha(Theme.primary, 0.12)
                            : item.hovered ? Qt.alpha(Theme.onSurface, 0.08)
                            : "transparent"
                    }

                    HoverHandler { cursorShape: Qt.PointingHandCursor }
                }
            }
        }

        Rectangle {
            Layout.preferredWidth: 1
            Layout.fillHeight: true
            color: Qt.alpha(Theme.onSurface, 0.12)
        }

        Flickable {
            id: flickable
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            contentHeight: loader.height + 56
            boundsBehavior: Flickable.StopAtBounds

            ScrollBar.vertical: ScrollBar {}

            Loader {
                id: loader
                x: 32
                y: 28
                width: flickable.width - 64
                height: implicitHeight
                source: root.categoryComponents[root.current]
                onLoaded: flickable.contentY = 0
            }
        }
    }
}
