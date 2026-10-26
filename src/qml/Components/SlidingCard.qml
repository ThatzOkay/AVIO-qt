import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

Pane {
    id: panel

    property bool shown: false
    property bool open: false
    property real topOffset: 70
    property Item target
    default property alias content: layout.data 

    function toggle() { open = !open }
    onShownChanged: open = shown
    onOpenChanged: if (open) align()

    function align() {
        const left = target ? target.mapToItem(parent, 0, 0).x : 0
        x = Math.max(12, Math.min(left, parent.width - width - 12))
    }

    parent: Overlay.overlay
    z: 1000
    width: Math.min(450, parent.width - 24)
    padding: 16

    y: topOffset
    visible: open

    contentItem: ColumnLayout {
        id: layout
        spacing: 8
    }

    background: Rectangle {
        color: Material.dialogColor
        radius: 8
        border.width: 1
        border.color: Qt.alpha(Material.foreground, 0.2)
    }
}