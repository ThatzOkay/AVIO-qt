import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt
import "../Components"

Item {
    id: root
    property string title: "Home"

    // The phone showed its own UI instead of projecting
    Button {
        visible: StatusStore.aaStatus === "host-ui"
        anchors.centerIn: parent
        highlighted: true
        font.pixelSize: 20
        text: qsTr("Resume Android Auto")
        onClicked: StatusStore.resume()
    }

    Label {
        visible: StatusStore.aaStatus !== "host-ui"
        anchors.centerIn: parent
        font.pixelSize: 24
        font.bold: true
        text: "Connect phone to start Android Auto"
    }
}
