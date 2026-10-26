import QtQuick
import QtQuick.Effects
import avio_qt

Item {
    id: root
    property url source
    property int size: 24
    property color color: Theme.primary

    implicitWidth: size
    implicitHeight: size

    Image {
        id: image
        anchors.fill: parent
        source: root.source
        sourceSize: Qt.size(root.size * 2, root.size * 2)
        visible: false
    }
    MultiEffect {
        anchors.fill: parent
        source: image
        colorization: 1
        colorizationColor: root.color
    }
}
