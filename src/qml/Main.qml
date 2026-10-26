import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import avio_qt
import "Components"
import "Pages"

ApplicationWindow {
    id: root
    // From Rust (window::startup): the persisted setting or the AVIO_KIOSK override
    required property bool kiosk
    property string route: "/"
    // While Android Auto is projecting the video plane sits underneath this window, so the
    // window has to let it show through
    readonly property bool projecting: StatusStore.aaStatus === "connected"

    visible: true
    width:  Screen.width
    height:  Screen.height
    visibility: kiosk ? Window.FullScreen : Window.Windowed
    // Transparent from the start (the alpha channel has to exist when the window is created); the
    // opaque app background below is what hides it, except while projecting
    color: "transparent"
    background: Rectangle {
        color: root.projecting ? "transparent" : Theme.surface
    }

    onWidthChanged: StatusStore.set_display_size(width, height)
    onHeightChanged: StatusStore.set_display_size(width, height)

    // The window manager (or a stray keypress) can knock a kiosk window out of fullscreen
    onVisibilityChanged: {
        if (kiosk && visibility !== Window.FullScreen && visibility !== Window.Hidden)
            showFullScreen()
    }

    // Rust asks for focus back after the video toplevel (a separate surface) steals it
    Connections {
        target: StatusStore
        function onFocus_requested() { root.requestActivate() }
    }

    // The stores read the system state and register for updates from the background worker
    Component.onCompleted: {
        SettingsStore.init()
        StatusStore.init()
        RadioStore.init()
        UsbService.init()
        UdevStore.init()
    }
    title: qsTr("AVIO")

    Material.theme: Theme.dark ? Material.Dark : Material.Light
    Material.background: Theme.surface
    Material.foreground: Theme.onSurface
    Material.accent: Theme.primary
    Material.primary: Theme.surface


    header: TopBar {
        visible: !root.projecting
        onNavigate: (path) => root.route = path
        Layout.fillWidth: true
        title: pages.children[pages.currentIndex] ? pages.children[pages.currentIndex].title : ""
    }

    footer: BottomBar {
        visible: !root.projecting
        route: root.route
        onNavigate: (path) => root.route = path
    }

    // avio's main area: a rounded surface-container card inset from the bars
    Rectangle {
        visible: !root.projecting
        anchors.fill: parent
        anchors.topMargin: 16
        anchors.leftMargin: 16
        anchors.rightMargin: 16
        anchors.bottomMargin: 16
        radius: 28
        color: Theme.surfaceContainer
        clip: true

        StackLayout {
            id: pages
            anchors.fill: parent
            currentIndex: ["/", "/radio", "/media", "/settings"].indexOf(root.route)

            Index {}
            Radio {}
            Media {}
            Settings {}
        }
    }

    // While projecting this replaces the app shell: a transparent layer over the whole window that
    // lets the video plane underneath show through and captures touch for it. Points are
    // normalised so they don't depend on the window size.
    MultiPointTouchArea {
        anchors.fill: parent
        enabled: root.projecting
        visible: root.projecting

        onPressed: (points) => points.forEach(p => StatusStore.touch_down(p.pointId, p.x / width, p.y / height))
        onUpdated: (points) => points.forEach(p => StatusStore.touch_move(p.pointId, p.x / width, p.y / height))
        onReleased: (points) => points.forEach(p => StatusStore.touch_up(p.pointId, p.x / width, p.y / height))
    }

    UdevDialog {}
}
