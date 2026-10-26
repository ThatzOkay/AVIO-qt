import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import avio_qt

ColumnLayout {
    id: root
    readonly property var radio: RadioStore.state ?? ({})
    readonly property int stationCount: (radio.dab?.stations ?? []).length
    readonly property bool busy: radio.busy ?? false

    // A dialog's file:// URL as the path Rust wants
    function toPath(url) {
        return decodeURIComponent(url.toString().replace(/^file:\/\/(\/(?=[A-Za-z]:))?/, ""))
    }

    spacing: 12

    Label {
        text: "Radio"
        color: Theme.onSurface
        font.pixelSize: 24
        Layout.bottomMargin: 4
    }

    SettingsCard {
        SettingsSubheader { text: "DAB stations" }

        SettingsRow {
            iconSource: "../../../assets/icons/mdi/radio.png"
            title: "Export station list"
            subtitle: root.stationCount + " stations and the favorites, saved as avio-dab-stations.json in a folder you pick"
            interactive: !root.busy && root.stationCount > 0
            enabled: interactive
            onClicked: exportDialog.open()
        }
        SettingsDivider {}
        SettingsRow {
            iconSource: "../../../assets/icons/mdi/radio.png"
            title: "Import station list"
            subtitle: "Replaces the stations and favorites with the ones in a file you pick"
            interactive: !root.busy
            enabled: interactive
            onClicked: importDialog.open()
        }
    }

    Label {
        Layout.fillWidth: true
        visible: text.length > 0
        text: root.radio.error ?? root.radio.notice ?? ""
        color: root.radio.error ? Theme.error : Theme.onSurfaceVariant
        font.pixelSize: 14
        wrapMode: Text.WordWrap
    }

    FolderDialog {
        id: exportDialog
        title: "Export station list to"
        onAccepted: RadioStore.dab_export_list(root.toPath(selectedFolder))
    }

    FileDialog {
        id: importDialog
        title: "Import station list"
        nameFilters: ["Station lists (*.json)", "All files (*)"]
        onAccepted: RadioStore.dab_import_list(root.toPath(selectedFile))
    }
}
