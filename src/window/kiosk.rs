//! Kiosk/fullscreen mode: whether we want it. Main.qml puts the window into fullscreen (and
//! re-enters it if the window manager knocks it out) when this says yes.

/// Whether kiosk mode is wanted, from either the persisted setting or the `AVIO_KIOSK` env var
/// override.
pub fn want_kiosk(kiosk_setting: bool) -> bool {
    kiosk_setting || std::env::var("AVIO_KIOSK").as_deref() == Ok("1")
}
