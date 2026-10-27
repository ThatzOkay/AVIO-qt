mod audio;
mod projection;
mod commands;
mod paths;
mod runtime;
mod radio;
mod screen;
mod shared;
mod store;
mod usb;
mod video;
mod window;
mod theme;

use qtbridge::{QApp, qobject};
use store::radio_store::RadioStore;
use store::settings_store::SettingsStore;
use store::status_store::StatusStore;
use store::udev_store::UdevStore;
use theme::Theme;

use crate::usb::usb_service::UsbService;

const WORKER_THREADS: usize = 3;

// Debug builds read the QML from the source tree, so scripts/dev.sh can relaunch on an edit.
// Release builds carry it inside the binary, with the icons the QML refers to as ../../assets/icons
#[cfg(debug_assertions)]
const QML_MAIN: &str = concat!("file://", env!("CARGO_MANIFEST_DIR"), "/src/qml/Main.qml");
#[cfg(not(debug_assertions))]
const QML_MAIN: &str = "qrc:/qml/Main.qml";
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Default)]
pub struct Backend {}

#[qobject(Singleton)]
impl Backend {
    #[qslot]
    fn say_hello(&self) {
        println!("Hello World!")
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Linux kiosk: relaunches this binary inside the nested avio-compositor, the window that
    // matters lives in that child
    if video::compositor_bootstrap::maybe_bootstrap() {
        return Ok(());
    }

    unsafe { std::env::set_var("QT_QUICK_CONTROLS_STYLE", "Material") };

    #[cfg(not(debug_assertions))]
    {
        qtbridge::include_bytes_qml!("qml");
        qtbridge::include_bytes_qml!("assets/icons");
    }

    // Not #[tokio::main]: Qt's event loop must run on the main thread outside the runtime context
    // Surface RT has 4 cores: one for Qt's UI thread, three for async work
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKER_THREADS)
        .enable_all()
        .build()?;
    runtime::init(runtime.handle().clone());
    commands::start();

    // Before Qt creates the singletons, since they start from the saved values
    store::settings_store::load();
    radio::start();
    let settings = store::settings_store::snapshot();
    let kiosk = window::startup(settings.kiosk, &settings.display_mode);

    QApp::new()
        .set_initial_property("kiosk", &kiosk)
        .register::<UsbService>()
        .register::<Backend>()
        .register::<Theme>()
        .register::<SettingsStore>()
        .register::<RadioStore>()
        .register::<StatusStore>()
        .register::<UdevStore>()
        .load_qml_from_file(QML_MAIN)
        .run();

    // Qt's loop has ended: stop watching USB, tell the Android Auto session to release the phone
    // and the loopback port, and take the gst-host child down with us
    usb::usb_service::stop();
    projection::driver::aa::session_handle::shared().request_shutdown();
    runtime.block_on(async {
        video::gst_video::VideoRuntime::shared().shutdown().await;
        // Dropping the runtime next aborts whatever is still running, so give the session a
        // moment to close the USB bridge itself
        tokio::time::sleep(SHUTDOWN_GRACE).await;
    });
    Ok(())
}
