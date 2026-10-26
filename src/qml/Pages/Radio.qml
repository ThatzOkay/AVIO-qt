import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt
import "../Components/Radio"

Item {
    id: root
    property string title: "Radio"

    readonly property bool open: StackLayout.isCurrentItem
    readonly property bool hasFm: StatusStore.radioFm
    readonly property bool hasDab: StatusStore.radioDab !== ""
    property string tab: hasFm ? "fm" : "dab"

    function sync() {
        if (open && (hasFm || hasDab))
            RadioStore.activate(tab)
        else
            RadioStore.deactivate()
    }

    onOpenChanged: sync()
    onTabChanged: sync()
    onHasFmChanged: {
        if (!hasFm && tab === "fm" && hasDab)
            tab = "dab"
        else
            sync()
    }
    onHasDabChanged: {
        if (!hasDab && tab === "dab" && hasFm)
            tab = "fm"
        else
            sync()
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        TabBar {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: 16
            Layout.preferredWidth: 220
            visible: root.hasFm && root.hasDab
            background: null
            currentIndex: root.tab === "dab" ? 0 : 1
            onCurrentIndexChanged: root.tab = currentIndex === 0 ? "dab" : "fm"

            TabButton { text: "DAB" }
            TabButton { text: "FM" }
        }

        Loader {
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: root.hasFm || root.hasDab
            sourceComponent: root.tab === "fm" ? fmRadio : dabRadio
        }

        Label {
            Layout.alignment: Qt.AlignHCenter
            Layout.fillHeight: true
            visible: !root.hasFm && !root.hasDab
            verticalAlignment: Text.AlignVCenter
            text: "No radio hardware plugged in"
            color: Theme.onSurfaceVariant
            font.pixelSize: 16
        }
    }

    Component {
        id: fmRadio
        FmRadio {}
    }
    Component {
        id: dabRadio
        DabRadio {}
    }
}
