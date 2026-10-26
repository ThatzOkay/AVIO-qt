import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt

ItemDelegate {
    id: row
    property url iconSource
    property string title
    property string subtitle
    property bool selected: false
    property bool interactive: false
    property alias trailing: trailingSlot.data

    Layout.fillWidth: true
    Layout.leftMargin: 8
    Layout.rightMargin: 8
    implicitHeight: Math.max(72, contentItem.implicitHeight + 24)
    leftPadding: 16
    rightPadding: 16
    topPadding: 0
    bottomPadding: 0
    hoverEnabled: interactive

    background: Rectangle {
        radius: 8
        color: !row.interactive ? "transparent"
            : row.pressed ? Qt.alpha(Theme.onSurface, 0.12)
            : row.hovered ? Qt.alpha(Theme.onSurface, 0.08)
            : "transparent"
    }

    contentItem: RowLayout {
        spacing: 16

        SettingsIcon {
            source: row.iconSource
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            Label {
                Layout.fillWidth: true
                text: row.title
                color: Theme.onSurface
                font.pixelSize: 16
                elide: Text.ElideRight
            }
            Label {
                Layout.fillWidth: true
                visible: text.length > 0
                text: row.subtitle
                color: Theme.onSurfaceVariant
                font.pixelSize: 14
                wrapMode: Text.WordWrap
            }
        }

        Item {
            id: trailingSlot
            visible: children.length > 0
            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
        }

        SettingsIcon {
            visible: row.selected
            size: 20
            source: "../../../assets/icons/mdi/check-circle-outline.png"
        }
    }

    HoverHandler {
        enabled: row.interactive
        cursorShape: Qt.PointingHandCursor
    }
}
