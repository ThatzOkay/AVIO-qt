#[cxx::bridge]
mod ffi {
    struct CodecSupport {
        codec: String,
        hw: bool,
        sw: bool,
    }

    unsafe extern "C++" {
        include!("gst-video/src/native/gst_video.h");

        type Player;

        // `gv_` prefix: GStreamer/GLib own several bare global names we'd otherwise collide
        // with (e.g. `gst_version(guint*, guint*, guint*, guint*)`).
        fn gv_version() -> String;
        fn gv_probe_codecs() -> Vec<CodecSupport>;

        fn gv_create_player(codec: &str, handle: u64) -> UniquePtr<Player>;
        fn gv_start(player: Pin<&mut Player>);
        fn gv_push_buffer(player: Pin<&mut Player>, data: &[u8]) -> bool;
        fn gv_is_dead(player: &Player) -> bool;
        fn gv_set_visible(player: Pin<&mut Player>, visible: bool);
        #[allow(clippy::too_many_arguments)]
        fn gv_set_content_region(
            player: Pin<&mut Player>,
            crop_l: f64,
            crop_t: f64,
            vis_w: f64,
            vis_h: f64,
            tier_w: f64,
            tier_h: f64,
        );
        fn gv_set_gamma(
            player: Pin<&mut Player>,
            gamma: f64,
            contrast: f64,
            gain_r: f64,
            gain_g: f64,
            gain_b: f64,
        );
        fn gv_stop(player: Pin<&mut Player>);

        fn gv_set_backdrop(handle: u64, r: f64, g: f64, b: f64);

        #[cfg(target_os = "linux")]
        fn gv_run_host(sock_path: &str, crash_path: &str);
    }
}

// SAFETY: a `Player` is only ever accessed through `&mut` (via the safe `Player` wrapper below),
// so there's never concurrent access from multiple threads at once — only ever moved to a new
// owning thread between exclusive uses, which is what `Send` promises.
unsafe impl Send for ffi::Player {}

/// Hardware/software decode support for one codec, as reported by `probe_codecs`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CodecSupport {
    pub hw: bool,
    pub sw: bool,
}

/// Per-codec decode support on this platform's GStreamer install.
#[derive(Debug, Clone, Copy, Default)]
pub struct CodecProbe {
    pub h264: CodecSupport,
    pub h265: CodecSupport,
    pub vp9: CodecSupport,
    pub av1: CodecSupport,
}

/// The linked GStreamer version string (e.g. "GStreamer 1.24.0").
pub fn version() -> String {
    ffi::gv_version()
}

/// Probes which codecs this platform's GStreamer can decode, and whether hardware
/// acceleration is available for each.
pub fn probe_codecs() -> CodecProbe {
    let mut probe = CodecProbe::default();
    for entry in ffi::gv_probe_codecs() {
        let support = CodecSupport {
            hw: entry.hw,
            sw: entry.sw,
        };
        match entry.codec.as_str() {
            "h264" => probe.h264 = support,
            "h265" => probe.h265 = support,
            "vp9" => probe.vp9 = support,
            "av1" => probe.av1 = support,
            _ => {}
        }
    }
    probe
}

/// An in-process decode pipeline with an optional native video overlay (mac/Windows).
/// Dropping it tears down the pipeline and removes the overlay.
pub struct Player(cxx::UniquePtr<ffi::Player>);

impl Player {
    /// Creates the decode pipeline for `codec` ("h264"/"h265"/"vp9"/"av1"), attaching a native
    /// overlay to `handle` (an `NSView*`/`HWND`; pass 0 where no in-process overlay is needed).
    /// Returns `None` if the pipeline failed to parse.
    pub fn create(codec: &str, handle: u64) -> Option<Player> {
        let inner = ffi::gv_create_player(codec, handle);
        if inner.is_null() {
            None
        } else {
            Some(Player(inner))
        }
    }

    pub fn start(&mut self) {
        ffi::gv_start(self.0.pin_mut());
    }

    /// Pushes one encoded access unit into the pipeline. Returns `false` if the player has no
    /// appsrc yet or the buffer was empty.
    pub fn push_buffer(&mut self, data: &[u8]) -> bool {
        ffi::gv_push_buffer(self.0.pin_mut(), data)
    }

    /// True once the pipeline's sink has lost its output surface (e.g. the user closed a bare
    /// waylandsink window on a desktop with no avio-compositor to embed it) and needs to be
    /// recreated before it'll show anything again.
    pub fn is_dead(&self) -> bool {
        self.0.as_ref().map(ffi::gv_is_dead).unwrap_or(true)
    }

    pub fn set_visible(&mut self, visible: bool) {
        ffi::gv_set_visible(self.0.pin_mut(), visible);
    }

    /// Crops/positions the native overlay to a content region inside the decoded tier, so the
    /// user-chosen aspect ratio fills the display without stretching.
    #[allow(clippy::too_many_arguments)]
    pub fn set_content_region(
        &mut self,
        crop_l: f64,
        crop_t: f64,
        vis_w: f64,
        vis_h: f64,
        tier_w: f64,
        tier_h: f64,
    ) {
        ffi::gv_set_content_region(self.0.pin_mut(), crop_l, crop_t, vis_w, vis_h, tier_w, tier_h);
    }

    pub fn set_gamma(&mut self, gamma: f64, contrast: f64, gain_r: f64, gain_g: f64, gain_b: f64) {
        ffi::gv_set_gamma(self.0.pin_mut(), gamma, contrast, gain_r, gain_g, gain_b);
    }

    /// Stops playback and removes the native overlay immediately. The pipeline itself is fully
    /// torn down when the `Player` is dropped.
    pub fn stop(&mut self) {
        ffi::gv_stop(self.0.pin_mut());
    }
}

/// Paints the window's content view (below the video overlay) with a theme colour, where the UI
/// is transparent and no video covers it. macOS only; a no-op elsewhere.
pub fn set_backdrop(handle: u64, r: f64, g: f64, b: f64) {
    ffi::gv_set_backdrop(handle, r, g, b);
}

/// Runs the out-of-process gst-host: connects to `sock_path` and serves the create/push/stop/
/// setGamma protocol until the socket closes. Never returns. Linux only.
#[cfg(target_os = "linux")]
pub fn run_host(sock_path: &str, crash_path: &str) {
    ffi::gv_run_host(sock_path, crash_path);
}
