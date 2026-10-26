import QtQuick
import avio_qt

// Signal strength as growing bars; `bars` of `count` are lit
Row {
    id: root
    property int bars: 0
    property int count: 6
    property color color: Theme.primary

    spacing: 2

    Repeater {
        model: root.count
        delegate: Item {
            id: slot
            required property int index

            width: 4
            height: 5 + (root.count - 1) * 3

            Rectangle {
                anchors.bottom: parent.bottom
                width: parent.width
                height: 5 + slot.index * 3
                radius: 1
                color: slot.index < root.bars ? root.color : Qt.alpha(Theme.onSurfaceVariant, 0.3)
            }
        }
    }
}
