import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import avio_qt

// Asks before AVIO installs its USB permission (udev) rule, and reports how that went.
// The state machine lives in UdevStore; this only reflects it.
Dialog {
    id: self

    readonly property string udevState: UdevStore.state
    readonly property bool failed: udevState === "failed"
    readonly property bool installed: udevState === "installed"
    readonly property bool outdated: udevState === "outdated"

    anchors.centerIn: Overlay.overlay
    width: Math.min(560, Overlay.overlay ? Overlay.overlay.width - 64 : 560)
    modal: true
    closePolicy: Popup.NoAutoClose

    title: installed ? qsTr("Installation Complete")
         : failed ? qsTr("Installation Failed")
         : outdated ? qsTr("USB Permission Update")
         : qsTr("USB Permission Required")

    // Dialog.accepted / rejected carry the button the user picked
    footer: DialogButtonBox {
        Button {
            text: self.installed ? qsTr("Restart")
                : self.failed ? qsTr("Retry")
                : self.outdated ? qsTr("Update")
                : qsTr("Install")
            DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
        }
        Button {
            text: qsTr("Skip")
            visible: !self.installed
            DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
        }
    }

    contentItem: Label {
        wrapMode: Text.WordWrap
        text: self.installed ? qsTr("udev rule installed. AVIO will now restart to apply it.")
            : self.failed ? qsTr("Could not install the udev rule.\nThe authorization was cancelled or the password was wrong.")
            : self.outdated ? qsTr("AVIO needs to update its udev rule for USB device access.\n\nThe existing rule at %1 is outdated (wired Android Auto needs additional phone vendor entries). It will be replaced.").arg(UdevStore.ruleFile)
            : qsTr("AVIO needs permission to access USB devices.\n\nA udev rule will be installed to %1.").arg(UdevStore.ruleFile)
    }

    onAccepted: installed ? UdevStore.restart() : UdevStore.install()
    onRejected: UdevStore.skip()

    // "installing" has no dialog of its own: pkexec shows the polkit prompt
    onUdevStateChanged: {
        if (udevState === "missing" || udevState === "outdated" || udevState === "failed" || udevState === "installed")
            open()
        else
            close()
    }
}
