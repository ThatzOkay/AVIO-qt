import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import avio_qt
import "../Settings"

RowLayout {
    spacing: 6

    SettingsIcon {
        source: "../../../assets/icons/mdi/star.png"
        size: 14
        color: Theme.onSurfaceVariant
    }
    Label {
        text: "tap to play, hold to save"
        color: Theme.onSurfaceVariant
        font.pixelSize: 12
    }
}
