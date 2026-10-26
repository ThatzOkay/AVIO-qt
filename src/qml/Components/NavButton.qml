import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import avio_qt

Button {
    id: self
    required property string path
    property bool tinted: false
    property bool active: false

    Layout.preferredWidth: 220
    Layout.fillHeight: true
    flat: true
    // Material's default insets/padding shrink the highlight and clamp the icon
    topInset: 0
    bottomInset: 0
    leftInset: 0
    rightInset: 0
    // Material sets horizontalPadding/verticalPadding itself, which beat a plain `padding`
    horizontalPadding: 0
    verticalPadding: 0
    display: AbstractButton.IconOnly
    icon.width: 64
    icon.height: 64
    icon.color: tinted ? Theme.primary : "transparent"

    background: Rectangle {
        color: self.active ? Qt.alpha(Theme.primary, 0.12) : "transparent"
    }

    HoverHandler {
        cursorShape: Qt.PointingHandCursor
    }
}
