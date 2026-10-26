import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt

Pane {
    id: bar
    property string route
    signal navigate(string path)

    padding: 0
    implicitHeight: 70
    background: Rectangle { color: Theme.surface }

    RowLayout {
        anchors.horizontalCenter: parent.horizontalCenter
        height: parent.height
        spacing: 0

        NavButton {
            path: "/"
            active: bar.route === path
            onClicked: bar.navigate(path)
            icon.source: "../../assets/icons/android-auto.png"
        }
        NavButton {
            path: "/radio"
            visible: StatusStore.radioAvailable
            active: bar.route === path
            onClicked: bar.navigate(path)
            icon.source: "../../assets/icons/radio.png"
        }
        NavButton {
            path: "/media"
            visible: StatusStore.mediaAvailable
            tinted: true
            active: bar.route === path
            onClicked: bar.navigate(path)
            icon.source: "../../assets/icons/media.png"
        }
        NavButton {
            path: "/settings"
            tinted: true
            active: bar.route === path
            onClicked: bar.navigate(path)
            icon.source: "../../assets/icons/settings.png"
        }
    }
}
