//! Wraps the external `wlr-randr` CLI against the host Wayland compositor - only useful for
//! wlroots-based hosts (Sway/Hyprland/Cage/labwc), since that's the compositor family which
//! implements the `wlr-output-management-unstable-v1` protocol `wlr-randr` needs.

use std::process::Command;
use std::time::Duration;

use super::exec::run_with_timeout;

// Set by `compositor_bootstrap.rs` to whatever WAYLAND_DISPLAY pointed at *before* it decided to
// nest - `wlr-randr` needs to target that host session, not the nested display this process is
// actually running inside.
const HOST_DISPLAY_ENV: &str = "AVIO_HOST_WAYLAND_DISPLAY";

// External tool, outside our control - a hard ceiling so a stuck/unreachable Wayland connection
// can't hang the "list display modes" feature indefinitely.
const TIMEOUT: Duration = Duration::from_secs(3);

fn run(args: &[&str]) -> Option<String> {
    let host_display = std::env::var(HOST_DISPLAY_ENV).ok()?;
    let mut cmd = Command::new("wlr-randr");
    cmd.env("WAYLAND_DISPLAY", host_display).args(args);
    let output = run_with_timeout(cmd, TIMEOUT)?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn host_output_name() -> Option<String> {
    let listing = run(&[])?;
    listing.lines().next()?.split_whitespace().next().map(str::to_string)
}

pub fn list_modes() -> Vec<String> {
    let Some(listing) = run(&[]) else {
        return Vec::new();
    };
    let mut modes: Vec<(u32, u32)> = Vec::new();
    for line in listing.lines() {
        let mut parts = line.split_whitespace();
        let Some(res) = parts.next() else { continue };
        let Some(px_token) = parts.next() else { continue };
        if !px_token.starts_with("px") {
            continue;
        }
        let Some((w, h)) = res.split_once('x') else {
            continue;
        };
        let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) else {
            continue;
        };
        if !modes.contains(&(w, h)) {
            modes.push((w, h));
        }
    }
    modes.sort_by_key(|(w, h)| std::cmp::Reverse(w * h));
    modes.into_iter().map(|(w, h)| format!("{w}x{h}")).collect()
}

/// Returns whether it actually applied `mode` - `false` means "try the next backend", not
/// necessarily an error (e.g. `wlr-randr`/a wlroots host just isn't present).
pub fn apply_mode(mode: &str) -> bool {
    if !list_modes().iter().any(|m| m == mode) {
        return false;
    }
    let Some(name) = host_output_name() else {
        return false;
    };
    run(&["--output", &name, "--mode", mode]).is_some()
}
