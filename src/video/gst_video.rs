use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use tokio::sync::Mutex;

use crate::video::gst_host::{self, GstHost, GstHostHandle};

// The real `CompositorControl` (talks to avio-compositor over a unix socket) only makes sense on
// Linux - every call into it is already gated behind `use_host_process()`
// (`cfg!(target_os = "linux")`), since mac/Windows render in-process with no compositor to talk
// to. Its actual implementation lives in gst_video_linux.rs (no cfg noise needed inside that file
// at all, since it's simply never compiled elsewhere); everywhere else gets the trivial no-op
// stub below.
#[cfg(target_os = "linux")]
#[path = "gst_video_linux.rs"]
mod gst_video_linux;
#[cfg(target_os = "linux")]
use gst_video_linux::CompositorControl;

#[cfg(not(target_os = "linux"))]
#[derive(Default)]
struct CompositorControl;

#[cfg(not(target_os = "linux"))]
impl CompositorControl {
    fn new() -> Self {
        Self
    }
    async fn claim(&mut self, _tag: &str) {}
    #[allow(clippy::too_many_arguments)]
    async fn videocfg(
        &mut self,
        _tag: &str,
        _screen: &str,
        _crop_l: f64,
        _crop_t: f64,
        _vis_w: f64,
        _vis_h: f64,
        _tier_w: f64,
        _tier_h: f64,
    ) {
    }
    async fn videoshow(&mut self, _tag: &str, _visible: bool) {}
    async fn screen(&mut self, _role: &str, _on: bool, _size: Option<(f64, f64)>) {}
    async fn set_backdrop(&mut self, _hex: &str) {}
    async fn gamma(&mut self, _g: GammaState) {}
    async fn restart(&mut self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GstVideoCodec {
    H264,
    H265,
    VP9,
    AV1,
}

impl GstVideoCodec {
    fn as_str(self) -> &'static str {
        match self {
            GstVideoCodec::H264 => "h264",
            GstVideoCodec::H265 => "h265",
            GstVideoCodec::VP9 => "vp9",
            GstVideoCodec::AV1 => "av1",
        }
    }
}

// Parse "#rrggbb" into 0..255 channels, falls back to black on a malformed value.
fn hex_to_rgb255(hex: Option<&str>) -> (u8, u8, u8) {
    let s = hex.unwrap_or("").trim();
    let s = s.strip_prefix('#').unwrap_or(s);

    if s.len() != 6 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return (0, 0, 0);
    }

    let n = u32::from_str_radix(s, 16).unwrap_or(0);
    (
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
    )
}

// Resolve the active backdrop colour for a config, falling back to the theme defaults.
pub fn backdrop_hex(dark_mode: bool, dark: Option<&str>, light: Option<&str>) -> String {
    let chosen = if dark_mode { dark } else { light };
    match chosen.filter(|s| !s.is_empty()) {
        Some(hex) => hex.to_string(),
        None => (if dark_mode { "#000000" } else { "#d4d4d4" }).to_string(),
    }
}

// Linux only: the pipeline lives in the gst-host process, with its own GLib main loop (so
// waylandsink resizes live) and isolated from a decoder crash. mac/Windows render in-process.
const fn use_host_process() -> bool {
    cfg!(target_os = "linux")
}

// mac/Windows only: point the in-process GStreamer at the app's bundled runtime, once.
fn prepare_runtime() {
    static PREPARED: std::sync::Once = std::sync::Once::new();
    PREPARED.call_once(|| {
        let root = if cfg!(target_os = "windows") {
            crate::audio::gstreamer::resolve_gstreamer_root()
        } else if cfg!(target_os = "macos") {
            // Only override when a bundled runtime is actually present; otherwise keep using
            // whatever GStreamer install the dev machine already has (e.g. Homebrew).
            crate::audio::gstreamer::resolve_gstreamer_root().filter(|r| r.exists())
        } else {
            None
        };
        let Some(root) = root else { return };

        // SAFETY: called once, from `Once::call_once`, before any in-process GStreamer pipeline
        // is created (gst_init reads these on first use), so there is no concurrent env access.
        unsafe {
            std::env::set_var("GST_PLUGIN_SYSTEM_PATH", "");
            std::env::set_var("GST_PLUGIN_PATH", root.join("lib").join("gstreamer-1.0"));
            let scanner_name = if cfg!(windows) {
                "gst-plugin-scanner.exe"
            } else {
                "gst-plugin-scanner"
            };
            std::env::set_var(
                "GST_PLUGIN_SCANNER",
                root.join("libexec")
                    .join("gstreamer-1.0")
                    .join(scanner_name),
            );
            if cfg!(windows) {
                let mut path = std::ffi::OsString::from(root.join("bin"));
                if let Some(existing) = std::env::var_os("PATH") {
                    path.push(";");
                    path.push(existing);
                }
                std::env::set_var("PATH", path);
            }
        }
    });
}

// Which codecs the loaded GStreamer can decode on this platform, and whether hardware
// acceleration is available for each.
pub fn probe_gst_codecs() -> gst_video::CodecProbe {
    prepare_runtime();
    gst_video::probe_codecs()
}

#[derive(Debug, Clone, Copy)]
pub struct GammaState {
    pub gamma: f64,
    pub contrast: f64,
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl GammaState {
    const NEUTRAL: GammaState = GammaState {
        gamma: 1.0,
        contrast: 1.0,
        r: 1.0,
        g: 1.0,
        b: 1.0,
    };
}

/// Shared video runtime: the out-of-process gst-host handle (Linux), the avio-compositor
/// control channel (Linux), and the current display calibration. One instance for the whole
/// app, reached through [`VideoRuntime::shared`] wherever a video surface (a window, or
/// eventually an Android Auto/CarPlay projection surface) needs to drive GStreamer.
pub struct VideoRuntime {
    gst_host: GstHostHandle,
    compositor: Mutex<CompositorControl>,
    gamma: Mutex<GammaState>,
}

impl Default for VideoRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoRuntime {
    pub fn shared() -> Arc<VideoRuntime> {
        static SHARED: std::sync::LazyLock<Arc<VideoRuntime>> =
            std::sync::LazyLock::new(|| Arc::new(VideoRuntime::new()));
        SHARED.clone()
    }

    pub fn new() -> Self {
        Self {
            gst_host: gst_host::new_handle(),
            compositor: Mutex::new(CompositorControl::new()),
            gamma: Mutex::new(GammaState::NEUTRAL),
        }
    }

    pub async fn set_compositor_backdrop(&self, hex: &str) {
        self.compositor.lock().await.set_backdrop(hex).await;
    }

    pub async fn set_compositor_screen(&self, role: &str, on: bool, size: Option<(f64, f64)>) {
        self.compositor.lock().await.screen(role, on, size).await;
    }

    pub async fn compositor_restart(&self) -> bool {
        self.compositor.lock().await.restart().await
    }

    pub async fn current_gamma(&self) -> GammaState {
        *self.gamma.lock().await
    }

    // Push the display calibration into every live video pipeline's glshader pass (all
    // platforms). Callers holding a live `GstVideo` should call `apply_gamma` on it afterwards
    // to push the new values into its in-process player immediately.
    pub async fn set_stream_gamma(&self, g: GammaState) {
        *self.gamma.lock().await = g;
        if use_host_process() {
            self.compositor.lock().await.gamma(g).await;
        }
    }

    /// Kills the gst-host child process, if one is running.
    pub async fn shutdown(&self) {
        GstHost::shutdown(&self.gst_host).await;
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ContentRegion {
    crop_l: f64,
    crop_t: f64,
    vis_w: f64,
    vis_h: f64,
    tier_w: f64,
    tier_h: f64,
}

static NEXT_PLAYER_ID: AtomicU32 = AtomicU32::new(1);

/// GStreamer video player. On Linux the pipeline lives in the gst-host process and this only
/// holds an id for it; on mac/Windows it drives the in-process gst-video crate directly.
///
/// `role` is the compositor tag for this plane (Linux); `target_screen` is which screen it's
/// placed on.
pub struct GstVideo {
    id: u32,
    runtime: Arc<VideoRuntime>,
    // Native view/window handle the in-process pipeline (mac/Windows) renders into. Unused on
    // Linux, where the gst-host process creates its own toplevel for the compositor to place.
    native_handle: u64,
    role: String,
    target_screen: String,
    started: bool,
    player: Option<gst_video::Player>,
    codec: Option<GstVideoCodec>,
    visible: bool,
    region: Option<ContentRegion>,
}

impl GstVideo {
    pub fn new(
        runtime: Arc<VideoRuntime>,
        native_handle: u64,
        role: impl Into<String>,
        target_screen: impl Into<String>,
    ) -> Self {
        Self {
            id: NEXT_PLAYER_ID.fetch_add(1, Ordering::Relaxed),
            runtime,
            native_handle,
            role: role.into(),
            target_screen: target_screen.into(),
            started: false,
            player: None,
            codec: None,
            visible: true,
            region: None,
        }
    }

    async fn ensure(&mut self, codec: GstVideoCodec) {
        if use_host_process() {
            if self.started && self.codec == Some(codec) {
                return;
            }
            self.dispose().await;
            // Tag the waylandsink toplevel the host process creates next.
            self.runtime.compositor.lock().await.claim(&self.role).await;
            GstHost::create_player(&self.runtime.gst_host, self.id, codec.as_str()).await;
            self.codec = Some(codec);
            self.started = true;
            self.apply_gamma().await;
            return;
        }

        prepare_runtime();
        if self.player.as_ref().is_some_and(|p| p.is_dead()) {
            // The sink lost its output surface; drop it so the check below recreates a fresh one.
            self.player = None;
            self.codec = None;
        }
        if self.player.is_some() && self.codec == Some(codec) {
            return;
        }
        self.dispose().await;
        let handle = self.native_handle;
        if handle == 0 {
            return;
        }
        self.runtime.compositor.lock().await.claim(&self.role).await;
        let Some(mut player) = gst_video::Player::create(codec.as_str(), handle) else {
            return;
        };
        player.start();
        player.set_visible(self.visible);
        if let Some(r) = self.region {
            player.set_content_region(r.crop_l, r.crop_t, r.vis_w, r.vis_h, r.tier_w, r.tier_h);
        }
        self.player = Some(player);
        self.codec = Some(codec);
        self.apply_gamma().await;
    }

    pub async fn push(&mut self, codec: GstVideoCodec, nal: &[u8]) {
        self.ensure(codec).await;
        if use_host_process() {
            if self.started {
                GstHost::push_buffer(&self.runtime.gst_host, self.id, nal).await;
            }
            return;
        }
        if let Some(player) = self.player.as_mut() {
            player.push_buffer(nal);
        }
    }

    // Apply the current calibration to this player's glshader pass. Re-sent after each (re)create.
    pub async fn apply_gamma(&mut self) {
        let g = self.runtime.current_gamma().await;
        if use_host_process() {
            self.runtime.compositor.lock().await.gamma(g).await;
            return;
        }
        if let Some(player) = self.player.as_mut() {
            player.set_gamma(g.gamma, g.contrast, g.r, g.g, g.b);
        }
    }

    // Show/hide the video surface as the user navigates in/out of projection.
    //
    // On Linux (host_process mode) without avio-compositor running, `videoshow` below is a
    // no-op (see CompositorControl::enabled) and there's no in-process `self.player` either —
    // meaning hiding had *no effect at all* in that setup: the last decoded frame just stayed
    // on screen frozen. Tear the gst-host player down instead when hiding without a compositor;
    // `push()` transparently recreates it (via `ensure()`) once frames resume.
    pub async fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        self.runtime
            .compositor
            .lock()
            .await
            .videoshow(&self.role, visible)
            .await;
        if let Some(player) = self.player.as_mut() {
            player.set_visible(visible);
        }
        if use_host_process() && !visible && self.started {
            GstHost::stop(&self.runtime.gst_host, self.id).await;
            self.started = false;
            self.codec = None;
        }
    }

    // Set the AA content region inside the decoded tier. The native view crops to it by
    // sizing + positioning the GL render (zero-copy).
    #[allow(clippy::too_many_arguments)]
    pub async fn set_content_region(
        &mut self,
        crop_l: f64,
        crop_t: f64,
        vis_w: f64,
        vis_h: f64,
        tier_w: f64,
        tier_h: f64,
    ) {
        self.region = (vis_w > 0.0 && vis_h > 0.0).then_some(ContentRegion {
            crop_l,
            crop_t,
            vis_w,
            vis_h,
            tier_w,
            tier_h,
        });
        // Linux: the compositor places + crops the tagged plane on its target screen.
        self.runtime
            .compositor
            .lock()
            .await
            .videocfg(
                &self.role,
                &self.target_screen,
                crop_l,
                crop_t,
                vis_w,
                vis_h,
                tier_w,
                tier_h,
            )
            .await;
        if let Some(player) = self.player.as_mut() {
            let r = self.region.unwrap_or_default();
            player.set_content_region(r.crop_l, r.crop_t, r.vis_w, r.vis_h, r.tier_w, r.tier_h);
        }
    }

    pub async fn dispose(&mut self) {
        if use_host_process() {
            if self.started {
                GstHost::stop(&self.runtime.gst_host, self.id).await;
            }
            self.started = false;
            self.codec = None;
            return;
        }
        if let Some(mut player) = self.player.take() {
            player.stop();
        }
        self.codec = None;
    }
}
