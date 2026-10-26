use std::collections::HashMap;

use tokio::{io::AsyncWriteExt, net::UnixStream};

use super::{hex_to_rgb255, use_host_process, GammaState};

// Control channel to avio-compositor. Video planes are addressed by tag (claim), then placed
// (videocfg) and toggled (videoshow). `state` is resent on reconnect.
#[derive(Default)]
pub(super) struct CompositorControl {
    path: Option<String>,
    sock: Option<UnixStream>,
    state: HashMap<String, String>, // resent every flush
    outbox: Vec<String>,            // one-shot lines, sent once
}

impl CompositorControl {
    pub(super) fn new() -> Self {
        Self {
            path: std::env::var("AVIO_COMPOSITOR_CTRL")
                .ok()
                .filter(|s| !s.is_empty()),
            ..Default::default()
        }
    }

    fn enabled(&self) -> bool {
        use_host_process() && self.path.is_some()
    }

    async fn flush(&mut self) {
        if !self.enabled() {
            self.outbox.clear();
            return;
        }
        if self.sock.is_none() {
            if let Some(path) = &self.path {
                self.sock = UnixStream::connect(path).await.ok();
            }
        }
        let Some(sock) = self.sock.as_mut() else {
            return;
        };
        for line in self.outbox.drain(..) {
            if let Err(e) = sock.write_all(line.as_bytes()).await {
                eprintln!("[CompositorControl] write failed: {e}, will reconnect on next flush");
                self.sock = None;
                return;
            }
        }
        for line in self.state.values() {
            if let Err(e) = sock.write_all(line.as_bytes()).await {
                eprintln!("[CompositorControl] write failed: {e}, will reconnect on next flush");
                self.sock = None;
                return;
            }
        }
    }

    // The next new video toplevel gets this tag. Send before creating the waylandsink.
    pub(super) async fn claim(&mut self, tag: &str) {
        if !self.enabled() {
            return;
        }
        self.outbox.push(format!("claim {tag}\n"));
        self.flush().await;
    }

    // Place + crop the tagged plane on a screen (fullscreen with its own AA content region).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn videocfg(
        &mut self,
        tag: &str,
        screen: &str,
        crop_l: f64,
        crop_t: f64,
        vis_w: f64,
        vis_h: f64,
        tier_w: f64,
        tier_h: f64,
    ) {
        if !self.enabled() {
            return;
        }
        let n = |v: f64| v.round() as i64;
        self.state.insert(
            format!("cfg:{tag}"),
            format!(
                "videocfg {tag} {screen} {} {} {} {} {} {}\n",
                n(crop_l),
                n(crop_t),
                n(vis_w),
                n(vis_h),
                n(tier_w),
                n(tier_h)
            ),
        );
        self.flush().await;
    }

    // Toggle the tagged plane's visibility.
    pub(super) async fn videoshow(&mut self, tag: &str, visible: bool) {
        if !self.enabled() {
            return;
        }
        self.state.insert(
            format!("show:{tag}"),
            format!("videoshow {tag} {}\n", visible as u8),
        );
        self.flush().await;
    }

    // Open/close a role's nested output (its own movable host window). Resent on reconnect.
    pub(super) async fn screen(&mut self, role: &str, on: bool, size: Option<(f64, f64)>) {
        if !self.enabled() {
            return;
        }
        let size = match size {
            Some((w, h)) if w > 0.0 && h > 0.0 => {
                format!(" {} {}", w.round() as i64, h.round() as i64)
            }
            _ => String::new(),
        };
        self.state.insert(
            format!("screen:{role}"),
            format!("screen {role} {}{size}\n", on as u8),
        );
        self.flush().await;
    }

    // Theme background for the compositor backdrop, hex "#rrggbb" from config.
    pub(super) async fn set_backdrop(&mut self, hex: &str) {
        if !self.enabled() {
            return;
        }
        let (r, g, b) = hex_to_rgb255(Some(hex));
        self.state.insert(
            "__backdrop__".to_string(),
            format!("backdrop {r} {g} {b}\n"),
        );
        self.flush().await;
    }

    // Push the display calibration to the compositor's per-video shader pass.
    pub(super) async fn gamma(&mut self, g: GammaState) {
        if !self.enabled() {
            return;
        }
        self.state.insert(
            "__gamma__".to_string(),
            format!("gamma {} {} {} {} {}\n", g.gamma, g.contrast, g.r, g.g, g.b),
        );
        self.flush().await;
    }

    // Ask the compositor to relaunch its inner UI child. One-shot, not resent on reconnect.
    pub(super) async fn restart(&mut self) -> bool {
        if !self.enabled() {
            return false;
        }
        self.outbox.push("restart\n".to_string());
        self.flush().await;
        true
    }
}
