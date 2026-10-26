//! Live USB hotplug watcher, replacing the need for the frontend to poll
//! `list_devices` (or a "check again" button) to notice the dongle
//! appearing/disappearing -- and auto-opens it once it does. Directly
//! serves this chip's actual deployment context (car-stereo head units
//! whose USB port loses power every ignition cycle, per `DabState`'s own
//! doc comment): without this, the driver would have to reach over and
//! tap "open" after every single ignition cycle instead of the app just
//! picking the device back up on its own.
//!
//! `nusb::watch_devices()`'s API and the pattern of consuming its
//! `Stream` in a background task were confirmed against a real, already-
//! working implementation (the user's sibling `avio` project's
//! `src-tauri/src/usb/usb_service.rs`, pinned to the same `nusb = "0.2"`
//! line this crate uses) rather than guessed -- that one runs on a
//! dedicated OS thread via `futures_lite::stream::block_on`; this one
//! uses a plain `tauri::async_runtime::spawn` task instead since
//! `HotplugWatch: Stream + Send` needs nothing thread-specific to poll,
//! and every other background task in this plugin (`spawn_event_forwarders`,
//! the core's own device task) is already tokio-task-shaped, not a raw
//! thread.

use futures_lite::StreamExt;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::commands::{device_present, open_device};
use crate::state::DabState;

/// Mirrors this plugin's other `<PLUGIN>://<name>` event naming
/// (`wradio-c100://snapshot`, `.../scan-progress`, ...) -- see
/// `guest-js/index.ts`'s own `cmd`/event-name helper for the pattern.
const PRESENCE_EVENT: &str = "wradio-c100://device-presence";

/// Spawns the watcher. Call once from `Builder::setup` -- like the core's
/// own device task, this runs for the app's whole lifetime, independent
/// of whether a device has ever been opened yet.
pub fn spawn<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let mut watcher = match nusb::watch_devices() {
            Ok(w) => w,
            Err(err) => {
                tracing::warn!(
                    ?err,
                    "hotplug: watch_devices failed -- device presence will only ever update via an explicit list_devices call"
                );
                return;
            }
        };

        // Not derived from the `Connected`/`Disconnected` event payloads
        // directly -- `Disconnected` only carries an opaque `DeviceId`,
        // not the vendor/product ids needed to tell "our dongle left"
        // from "literally any other USB device on the system changed"
        // (the id can't be looked up anymore once the device is gone).
        // Re-polling `device_present()` on every event and diffing
        // against what we last saw is simpler and just as correct --
        // hotplug events are rare enough that the extra poll is free.
        let mut present = device_present().await;
        if present {
            try_auto_open(&app).await;
        }

        while watcher.next().await.is_some() {
            let now_present = device_present().await;
            if now_present == present {
                continue;
            }
            present = now_present;
            let _ = app.emit(PRESENCE_EVENT, present);
            if present {
                try_auto_open(&app).await;
            }
        }
    });
}

/// Opens the device if it isn't already -- best-effort: a failure here
/// (e.g. another process, like a `scan_and_listen` session, already
/// holds the interface) is logged and left for the user to retry
/// manually via the still-present "Open device" button, not surfaced as
/// an alarming error for something the user didn't even ask for.
async fn try_auto_open<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<DabState>();
    if state.device.lock().await.is_some() {
        return;
    }
    if let Err(err) = open_device(app.clone(), state).await {
        tracing::warn!(?err, "hotplug: auto-open failed, leaving it for a manual retry");
    }
}
