import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt

ColumnLayout {
    id: root
    property color color
    signal picked(color value)

    property real hue: 0
    property real sat: 0
    property real val: 1
    readonly property bool dragging: squareArea.pressed || hueArea.pressed
    readonly property color current: Qt.hsva(hue, sat, val, 1)

    function sync() {
        if (dragging)
            return
        const c = Qt.color(root.color)
        if (c.hsvValue > 0 && c.hsvSaturation > 0)
            hue = c.hsvHue
        sat = c.hsvSaturation
        val = c.hsvValue
    }

    onColorChanged: sync()
    Component.onCompleted: sync()

    spacing: 12

    Rectangle {
        id: square
        Layout.fillWidth: true
        Layout.preferredHeight: 150
        radius: 8
        clip: true
        color: Qt.hsva(root.hue, 1, 1, 1)

        Rectangle {
            anchors.fill: parent
            gradient: Gradient {
                orientation: Gradient.Horizontal
                GradientStop { position: 0; color: "#FFFFFF" }
                GradientStop { position: 1; color: "#00FFFFFF" }
            }
        }
        Rectangle {
            anchors.fill: parent
            gradient: Gradient {
                GradientStop { position: 0; color: "#00000000" }
                GradientStop { position: 1; color: "#000000" }
            }
        }

        Rectangle {
            x: root.sat * square.width - width / 2
            y: (1 - root.val) * square.height - height / 2
            width: 20
            height: 20
            radius: 10
            color: root.current
            border.width: 3
            border.color: "white"
        }

        MouseArea {
            id: squareArea
            anchors.fill: parent
            cursorShape: Qt.CrossCursor
            function update(mouse) {
                root.sat = Math.max(0, Math.min(1, mouse.x / width))
                root.val = 1 - Math.max(0, Math.min(1, mouse.y / height))
            }
            onPressed: mouse => update(mouse)
            onPositionChanged: mouse => update(mouse)
            onReleased: root.picked(root.current)
        }
    }

    Rectangle {
        id: hueBar
        Layout.fillWidth: true
        Layout.preferredHeight: 16
        radius: 8
        gradient: Gradient {
            orientation: Gradient.Horizontal
            GradientStop { position: 0.0; color: "#FF0000" }
            GradientStop { position: 0.167; color: "#FFFF00" }
            GradientStop { position: 0.333; color: "#00FF00" }
            GradientStop { position: 0.5; color: "#00FFFF" }
            GradientStop { position: 0.667; color: "#0000FF" }
            GradientStop { position: 0.833; color: "#FF00FF" }
            GradientStop { position: 1.0; color: "#FF0000" }
        }

        Rectangle {
            x: root.hue * (hueBar.width - width)
            anchors.verticalCenter: parent.verticalCenter
            width: 20
            height: 20
            radius: 10
            color: Qt.hsva(root.hue, 1, 1, 1)
            border.width: 3
            border.color: "white"
        }

        MouseArea {
            id: hueArea
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            function update(mouse) {
                root.hue = Math.max(0, Math.min(1, mouse.x / width))
            }
            onPressed: mouse => update(mouse)
            onPositionChanged: mouse => update(mouse)
            onReleased: root.picked(root.current)
        }
    }

    RowLayout {
        spacing: 12

        Rectangle {
            implicitWidth: 32
            implicitHeight: 32
            radius: 16
            color: root.current
            border.width: 2
            border.color: Theme.outlineVariant
        }
        Label {
            text: root.current.toString().toUpperCase()
            color: Theme.onSurfaceVariant
            font.family: "monospace"
            font.pixelSize: 14
        }
    }
}
