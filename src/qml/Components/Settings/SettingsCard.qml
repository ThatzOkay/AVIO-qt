import QtQuick
import QtQuick.Layouts
import avio_qt

Rectangle {
    default property alias content: column.data

    Layout.fillWidth: true
    implicitHeight: column.implicitHeight + 16
    radius: 12
    color: Theme.surfaceContainerHigh

    ColumnLayout {
        id: column
        anchors.fill: parent
        anchors.topMargin: 8
        anchors.bottomMargin: 8
        spacing: 0
    }
}
