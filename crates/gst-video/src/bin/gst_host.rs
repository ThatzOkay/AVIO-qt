// Standalone gst-host executable: connects to the unix socket given as argv[1] and serves the
// create/push/stop/setGamma protocol until the socket closes. Spawned by gst_host.rs (in the
// avio Tauri app) so the pipeline runs outside the main process's GLib main loop.
#[cfg(target_os = "linux")]
fn main() {
    let mut args = std::env::args();
    let _exe = args.next();
    let sock_path = args.next().unwrap_or_default();
    let crash_path = args.next().unwrap_or_default();
    gst_video::run_host(&sock_path, &crash_path);
}

#[cfg(not(target_os = "linux"))]
fn main() {}
