//! Handle to whichever AA `Session` is currently running (if any), so commands invoked from the
//! frontend (touch input, buttons, ...) can reach it without needing a direct reference.
//! `wired_driver` registers/clears it as sessions start and end; only one session runs at a time.

use std::sync::{Arc, LazyLock};

use tokio::sync::{mpsc, Mutex, Notify};

use super::stack::session::session::SessionCommand;

// Falls back to the AA default tier until a session actually negotiates one (see
// `SessionConfig::default`'s video_width/video_height).
const DEFAULT_TOUCH_SIZE: (u32, u32) = (1280, 720);

pub struct AaSessionHandle {
    commands: Mutex<Option<mpsc::UnboundedSender<SessionCommand>>>,
    shutdown: Notify,
    // The touchscreen space actually advertised in the running session's SDR (tier minus
    // letterbox margin minus any user view-area inset) — touch commands must scale their
    // normalised 0..1 coordinates against this, not a hardcoded tier size, since it changes with
    // the user's resolution/view-area settings (see `commands::aa_send_pointer`).
    touch_size: Mutex<(u32, u32)>,
}

impl Default for AaSessionHandle {
    fn default() -> Self {
        Self {
            commands: Mutex::new(None),
            shutdown: Notify::new(),
            touch_size: Mutex::new(DEFAULT_TOUCH_SIZE),
        }
    }
}

/// The one handle for the whole app, shared by the wired driver and the input commands.
pub fn shared() -> Arc<AaSessionHandle> {
    static SHARED: LazyLock<Arc<AaSessionHandle>> = LazyLock::new(Arc::default);
    SHARED.clone()
}

impl AaSessionHandle {
    pub async fn set(&self, sender: mpsc::UnboundedSender<SessionCommand>) {
        *self.commands.lock().await = Some(sender);
    }

    pub async fn clear(&self) {
        *self.commands.lock().await = None;
        *self.touch_size.lock().await = DEFAULT_TOUCH_SIZE;
    }

    pub async fn set_touch_size(&self, width: u32, height: u32) {
        *self.touch_size.lock().await = (width, height);
    }

    pub async fn touch_size(&self) -> (u32, u32) {
        *self.touch_size.lock().await
    }

    /// Sends a command to the active session, if any. Returns `false` if there's no session
    /// running (or it just ended) — callers can treat that as a silent no-op.
    pub async fn send(&self, command: SessionCommand) -> bool {
        match self.commands.lock().await.as_ref() {
            Some(tx) => tx.send(command).is_ok(),
            None => false,
        }
    }

    /// Tells the wired driver (and whichever session is running) to stop and release the USB
    /// device/loopback port. `notify_one`, not `notify_waiters`: there's exactly one consumer at
    /// a time (the running session, or the bridge's connect loop between sessions), and
    /// `notify_one` stores a permit if it isn't waiting yet, so the signal isn't lost if it
    /// arrives between loop iterations.
    pub fn request_shutdown(&self) {
        self.shutdown.notify_one();
    }

    pub fn shutdown_notify(&self) -> &Notify {
        &self.shutdown
    }
}
