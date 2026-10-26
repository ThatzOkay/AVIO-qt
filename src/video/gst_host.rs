// Linux only: the pipeline lives in the gst-host process, with its own GLib main loop (so
// waylandsink resizes live) and isolated from a decoder crash, forwarding calls over a unix
// socket. mac/Windows render in-process instead (see `gst_video::use_host_process`), and tokio
// has no unix-socket support on Windows at all, so the real implementation lives in
// gst_host_linux.rs (no cfg noise needed inside that file at all, since it's simply never
// compiled elsewhere) and every other platform gets the no-op stub below with the same public
// API.
#[cfg(target_os = "linux")]
#[path = "gst_host_linux.rs"]
mod gst_host_linux;
#[cfg(target_os = "linux")]
pub use gst_host_linux::{new_handle, GstHost, GstHostHandle};

// mac/Windows: `gst_video::GstVideo` renders in-process (`use_host_process` is false), so every
// method here is dead weight at runtime - this stub exists only so call sites (in particular
// `VideoRuntime::shutdown`, which calls `GstHost::shutdown` unconditionally on every platform)
// don't need their own cfg-gating.
#[cfg(not(target_os = "linux"))]
mod stub {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    pub type GstHostHandle = Arc<Mutex<GstHost>>;

    pub fn new_handle() -> GstHostHandle {
        Arc::new(Mutex::new(GstHost))
    }

    #[derive(Default)]
    pub struct GstHost;

    impl GstHost {
        pub async fn shutdown(_handle: &GstHostHandle) {}

        pub async fn create_player(
            _handle: &GstHostHandle,
            _id: u32,
            _codec: &str,
        ) {
        }

        pub async fn push_buffer(
            _handle: &GstHostHandle,
            _id: u32,
            _nal: &[u8],
        ) {
        }

        pub async fn stop(_handle: &GstHostHandle, _id: u32) {}

        #[allow(clippy::too_many_arguments)]
        pub async fn set_gamma(
            _handle: &GstHostHandle,
            _id: u32,
            _gamma: f64,
            _contrast: f64,
            _r: f64,
            _g: f64,
            _b: f64,
        ) {
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub use stub::{new_handle, GstHost, GstHostHandle};
