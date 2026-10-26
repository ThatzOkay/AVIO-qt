import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt

RowLayout {
    id: root
    property url startIcon
    property url endIcon
    property alias from: slider.from
    property alias to: slider.to
    property alias stepSize: slider.stepSize
    property alias value: slider.value
    signal moved(real value)

    Layout.fillWidth: true
    spacing: 16

    SettingsIcon {
        visible: root.startIcon.toString() !== ""
        source: root.startIcon
        size: 18
        color: Theme.primary
    }
    Slider {
        id: slider
        Layout.fillWidth: true
        onMoved: root.moved(value)
    }
    SettingsIcon {
        visible: root.endIcon.toString() !== ""
        source: root.endIcon
        size: 18
        color: Theme.primary
    }
}
