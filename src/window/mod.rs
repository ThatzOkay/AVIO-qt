//! Main-window startup: kiosk vs windowed, and host display-mode switching before going kiosk.
//! The window itself is Main.qml's ApplicationWindow, which takes the answer from [`startup`] as
//! its `kiosk` property.

pub mod display_mode;
pub mod kiosk;

/// Called once from `main()`, before Qt creates the window. Applies the host display mode when
/// going kiosk, and returns whether the window should start in kiosk mode.
pub fn startup(kiosk_setting: bool, display_mode: &str) -> bool {
    let want_kiosk = kiosk::want_kiosk(kiosk_setting);

    if want_kiosk && !display_mode.is_empty() {
        display_mode::apply_host_output_mode(display_mode);
    }

    want_kiosk
}
