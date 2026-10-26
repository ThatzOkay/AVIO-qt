import QtQuick
import QtQuick.Controls
import avio_qt
import "../Settings"

Button {
    id: self
    property url iconSource
    property int diameter: 64
    property bool primary: false

    implicitWidth: diameter
    implicitHeight: diameter
    padding: 0
    topInset: 0
    bottomInset: 0
    leftInset: 0
    rightInset: 0
    opacity: enabled ? 1 : 0.38

    background: Rectangle {
        radius: width / 2
        color: self.primary ? Theme.primary : Theme.surfaceContainerHighest
        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: Theme.onSurface
            opacity: self.pressed ? 0.12 : self.hovered ? 0.08 : 0
        }
    }

    contentItem: Item {
        SettingsIcon {
            anchors.centerIn: parent
            source: self.iconSource
            size: self.primary ? 36 : 32
            color: self.primary ? Theme.onPrimary : Theme.onSurface
        }
    }

    HoverHandler {
        cursorShape: Qt.PointingHandCursor
    }
}
