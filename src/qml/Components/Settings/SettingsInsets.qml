import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import avio_qt

RowLayout {
    id: root
    property int insetTop
    property int insetBottom
    property int insetLeft
    property int insetRight
    signal edited(string side, int value)

    Layout.fillWidth: true
    spacing: 8

    Repeater {
        model: [
            { side: "top", prop: "insetTop", label: "Top" },
            { side: "bottom", prop: "insetBottom", label: "Bottom" },
            { side: "left", prop: "insetLeft", label: "Left" },
            { side: "right", prop: "insetRight", label: "Right" }
        ]
        delegate: TextField {
            id: field
            required property var modelData

            Layout.fillWidth: true
            Layout.preferredWidth: 1
            placeholderText: modelData.label
            text: String(root[modelData.prop])
            inputMethodHints: Qt.ImhDigitsOnly
            validator: IntValidator { bottom: 0 }
            Material.containerStyle: Material.Outlined
            onEditingFinished: root.edited(modelData.side, Number(text))
        }
    }
}
