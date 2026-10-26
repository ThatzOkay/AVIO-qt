import QtQuick
import QtQuick.Controls
import avio_qt
import "../Settings"

Button {
    id: self
    property url iconSource
    property int diameter: 64
    property bool primary: false
    // Shows a state (lit with `primary`) rather than offering an action: no hover or press
    // feedback, no pointer cursor
    property bool displayOnly: false

    implicitWidth: diameter
    implicitHeight: diameter
    // The Material style sets its own (asymmetric) left/right padding, which `padding` doesn't
    // override and which would push the icon off centre
    padding: 0
    leftPadding: 0
    rightPadding: 0
    topPadding: 0
    bottomPadding: 0
    topInset: 0
    bottomInset: 0
    leftInset: 0
    rightInset: 0
    opacity: !enabled ? 0.38 : displayOnly && !primary ? 0.6 : 1

    background: Rectangle {
        radius: width / 2
        color: self.primary ? Theme.primary : Theme.surfaceContainerHighest
        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: Theme.onSurface
            opacity: self.displayOnly ? 0 : self.pressed ? 0.12 : self.hovered ? 0.08 : 0
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
        cursorShape: self.displayOnly ? Qt.ArrowCursor : Qt.PointingHandCursor
    }
}
