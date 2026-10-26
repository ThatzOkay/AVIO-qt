//! Wraps KDE's `kscreen-doctor` CLI. It talks to the kscreen daemon over D-Bus, but - confirmed
//! via `ldd` (links `libQt6WaylandClient.so.6`/`libwayland-client.so.0`) - it *also* opens a real
//! Wayland connection through Qt's QPA platform plugin, using whatever `WAYLAND_DISPLAY` it
//! inherits. Left unset, that's our own nested `avio-compositor` (never the real host), and Qt's
//! platform init against a minimal compositor that doesn't implement whatever QPA expects fails
//! outright ("kscreen-doctor: Invalid config.") before it ever gets to the D-Bus part. So this
//! needs the exact same host-display override as `wlr_randr.rs`, not just a D-Bus session.

use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use super::exec::run_with_timeout;

// See `wlr_randr.rs` - same env var, same reason: `compositor_bootstrap.rs` sets this to whatever
// WAYLAND_DISPLAY pointed at *before* it decided to nest.
const HOST_DISPLAY_ENV: &str = "AVIO_HOST_WAYLAND_DISPLAY";

// `kscreen-doctor` talks to the kscreen daemon over D-Bus - normally near-instant, but this is an
// external tool outside our control, so it gets a hard ceiling rather than a chance to hang the
// whole "list display modes" feature indefinitely.
const TIMEOUT: Duration = Duration::from_secs(3);

// Points `cmd` at the host's Wayland session, never our own nested one - see the module doc for
// why kscreen-doctor needs this despite being "just" a D-Bus client.
fn target_host_display(cmd: &mut Command) {
    match std::env::var(HOST_DISPLAY_ENV) {
        Ok(host_display) => {
            cmd.env("WAYLAND_DISPLAY", host_display);
        }
        Err(_) => {
            // No host session at all (bare-metal DRM) - our own WAYLAND_DISPLAY still points at
            // avio-compositor, so drop it rather than let kscreen-doctor connect to ourselves.
            cmd.env_remove("WAYLAND_DISPLAY");
        }
    }
}

#[derive(Deserialize)]
struct ModeSize {
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct Mode {
    id: String,
    size: ModeSize,
    #[serde(rename = "refreshRate")]
    refresh_rate: f64,
}

#[derive(Deserialize)]
struct Output {
    name: String,
    connected: bool,
    enabled: bool,
    modes: Vec<Mode>,
}

#[derive(Deserialize)]
struct Doc {
    outputs: Vec<Output>,
}

fn query() -> Option<Doc> {
    let mut cmd = Command::new("kscreen-doctor");
    cmd.arg("-j");
    target_host_display(&mut cmd);
    let output = run_with_timeout(cmd, TIMEOUT)?;
    output.status.success().then_some(())?;
    serde_json::from_slice(&output.stdout).ok()
}

fn primary_output(doc: &Doc) -> Option<&Output> {
    doc.outputs
        .iter()
        .find(|o| o.connected && o.enabled)
        .or_else(|| doc.outputs.first())
}

pub fn list_modes() -> Vec<String> {
    let Some(doc) = query() else {
        return Vec::new();
    };
    let Some(out) = primary_output(&doc) else {
        return Vec::new();
    };
    let mut modes: Vec<(u32, u32)> = Vec::new();
    for m in &out.modes {
        let pair = (m.size.width, m.size.height);
        if !modes.contains(&pair) {
            modes.push(pair);
        }
    }
    modes.sort_by_key(|(w, h)| std::cmp::Reverse(w * h));
    modes.into_iter().map(|(w, h)| format!("{w}x{h}")).collect()
}

/// Returns whether it actually applied `mode` - `false` means "try the next backend", not
/// necessarily an error (e.g. `kscreen-doctor`/a KDE session just isn't present).
pub fn apply_mode(mode: &str) -> bool {
    let Some((w, h)) = mode.split_once('x') else {
        return false;
    };
    let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) else {
        return false;
    };
    let Some(doc) = query() else {
        return false;
    };
    let Some(out) = primary_output(&doc) else {
        return false;
    };
    // Prefer the highest refresh rate among modes matching the requested size.
    let Some(best) = out
        .modes
        .iter()
        .filter(|m| m.size.width == w && m.size.height == h)
        .max_by(|a, b| a.refresh_rate.total_cmp(&b.refresh_rate))
    else {
        return false;
    };

    let mut cmd = Command::new("kscreen-doctor");
    cmd.arg(format!("output.{}.mode.{}", out.name, best.id));
    target_host_display(&mut cmd);
    run_with_timeout(cmd, TIMEOUT).is_some_and(|o| o.status.success())
}
