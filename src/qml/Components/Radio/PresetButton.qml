import QtQuick
import QtQuick.Controls
import avio_qt

Item {
    id: root
    property string label
    property bool saved: false
    property bool active: false
    property int labelSize: 14
    signal tapped
    signal held

    property bool holdFired: false

    implicitWidth: 64
    implicitHeight: 64
    opacity: enabled ? 1 : 0.38

    Rectangle {
        anchors.fill: parent
        radius: width / 2
        color: Theme.surfaceContainerHighest
        border.width: root.active ? 2 : 0
        border.color: Theme.primary

        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: Theme.onSurface
            opacity: mouse.pressed ? 0.12 : 0
        }
    }

    Label {
        anchors.centerIn: parent
        width: 56
        horizontalAlignment: Text.AlignHCenter
        text: root.label
        color: Theme.onSurface
        font.pixelSize: root.labelSize
        font.bold: true
        elide: Text.ElideRight
    }

    Timer {
        id: holdTimer
        interval: 600
        onTriggered: {
            root.holdFired = true
            root.held()
        }
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        onPressed: {
            root.holdFired = false
            holdTimer.restart()
        }
        onReleased: {
            holdTimer.stop()
            if (!root.holdFired && root.saved)
                root.tapped()
        }
        onCanceled: holdTimer.stop()
    }
}
