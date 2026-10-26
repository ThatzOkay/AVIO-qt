import QtQuick
import QtQuick.Controls
import avio_qt

// Material 3 switch: off is an outlined track with a small handle, on is a filled primary track
Switch {
    id: control

    indicator: Rectangle {
        implicitWidth: 52
        implicitHeight: 32
        x: control.leftPadding
        y: (control.height - height) / 2
        radius: height / 2
        color: control.checked ? Theme.primary : Theme.surfaceContainerHighest
        border.width: 2
        border.color: control.checked ? Theme.primary : Theme.outline

        Rectangle {
            width: control.checked ? 24 : 16
            height: width
            radius: width / 2
            x: control.checked ? parent.width - width - 4 : 8
            anchors.verticalCenter: parent.verticalCenter
            color: control.checked ? Theme.onPrimary : Theme.outline
        }
    }

    HoverHandler {
        cursorShape: Qt.PointingHandCursor
    }
}
