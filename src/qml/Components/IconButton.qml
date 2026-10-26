import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import avio_qt

Button {
    id: self
    property bool active: false

    flat: true
    topInset: 0
    bottomInset: 0
    leftInset: 0
    rightInset: 0
    horizontalPadding: 16
    verticalPadding: 8
    display: AbstractButton.IconOnly
    icon.width: 32
    icon.height: 32
    icon.color: self.active ? Theme.primary : Theme.onSurface

    background: Rectangle {
        radius: height / 2
        color: self.active ? Qt.alpha(Theme.primary, 0.12) : "transparent"
    }

    HoverHandler {
        cursorShape: Qt.PointingHandCursor
    }
}
