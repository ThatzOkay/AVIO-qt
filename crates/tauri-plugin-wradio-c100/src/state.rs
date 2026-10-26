//! `tauri::State` wrapper around the core's [`DeviceHandle`], managed once
//! via `app.manage(...)` and shared across every command invocation.
//!
//! Starts empty (`None`) rather than opening the device in
//! `Builder::setup`: this chip's actual deployment context (car-stereo
//! head units whose USB port loses power every ignition cycle -- see
//! `RUST_PORT_PROMPT.md`'s Signal quality & robustness section) means
//! "device not present yet" has to be a normal, recoverable state the
//! app can start up into, not a hard failure at launch -- `open_device`
//! is its own command precisely so the frontend can retry it (and show
//! "please plug in your DAB receiver") instead of the whole app failing
//! to start when the dongle just isn't plugged in yet.

use tokio::sync::Mutex;
use wradio_c100_core::device::DeviceHandle;

use crate::playback::PlaybackHandle;

#[derive(Default)]
pub struct DabState {
    pub device: Mutex<Option<DeviceHandle>>,
    /// Set alongside `device` in `commands::open_device` (native audio
    /// playback needs a live `DeviceHandle` to subscribe audio frames
    /// from, so the two always start and end together) -- a separate
    /// field rather than bundling both into one `Option<(...)>` so
    /// existing `device`-only accessors (`commands::device_handle`,
    /// every plain tuning/scan command) didn't need to change shape for
    /// this.
    pub playback: Mutex<Option<PlaybackHandle>>,
}
