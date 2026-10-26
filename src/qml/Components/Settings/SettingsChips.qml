import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt

RowLayout {
    id: root
    property var options: []
    property string current
    signal selected(string value)

    spacing: 8

    Repeater {
        model: root.options
        delegate: Rectangle {
            id: chip
            required property var modelData
            readonly property bool active: root.current === modelData.value

            implicitWidth: chipLabel.implicitWidth + 32
            implicitHeight: 32
            radius: 16
            color: active ? Qt.alpha(Theme.primary, 0.12) : "transparent"
            border.width: 1
            border.color: active ? Theme.primary : Theme.onSurfaceVariant

            Label {
                id: chipLabel
                anchors.centerIn: parent
                text: chip.modelData.label
                color: chip.active ? Theme.primary : Theme.onSurfaceVariant
                font.pixelSize: 14
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: root.selected(chip.modelData.value)
            }
        }
    }
}
