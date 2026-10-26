#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::Command;

/// Linux only: spawn the nested `avio-compositor` and re-exec this same binary as its inner
/// Wayland client (its `-s` flag, see avio-compositor.c's getopt handling — forks+execs the
/// given command via `/bin/sh -c`, inheriting our env plus the new nested `WAYLAND_DISPLAY`).
///
/// Must run before `QApp::new()`, so this (outer) process never creates a real window. Call it
/// at the very top of `main()` and return immediately if it returns `true` — the window that
/// matters lives in the re-exec'd child once it's running inside the nested display, not in this
/// process.
#[cfg(target_os = "linux")]
pub fn maybe_bootstrap() -> bool {
    if std::env::var("AVIO_COMPOSITOR").as_deref() == Ok("1") {
        return false; // already the nested child
    }
    if std::env::var("AVIO_NO_COMPOSITOR").as_deref() == Ok("1") {
        return false; // opted out (desktop dev convenience)
    }

    let Some(launcher) = launcher_path() else {
        eprintln!("[compositor] avio-compositor bundle not found; running without it");
        return false;
    };

    let Ok(relaunch) = std::env::current_exe() else {
        eprintln!("[compositor] could not resolve current_exe() to relaunch inside the compositor");
        return false;
    };

    // QT_QPA_PLATFORM=wayland: without it Qt could fall back to XWayland (xcb) instead of the
    // nested display. AVIO_COMPOSITOR=1 short-circuits this same check on the re-exec'd instance.
    let inner = format!(
        "AVIO_COMPOSITOR=1 QT_QPA_PLATFORM=wayland '{}'",
        relaunch.display()
    );

    let ctrl_sock =
        std::env::temp_dir().join(format!("avio-compositor-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&ctrl_sock);

    // Must match the Wayland app_id the re-exec'd "main" window actually presents, so the
    // compositor recognizes it as the UI client rather than a generic (zero-sized) dialog — see
    // avio-compositor.c's AVIO_OUTPUT_APP_ID comparison. Qt derives the xdg_toplevel app_id from
    // the executable name, which is the package name.
    let app_id = env!("CARGO_PKG_NAME");

    let mut command = Command::new(&launcher);
    command
        .arg("-s")
        .arg(&inner)
        .env("AVIO_COMPOSITOR_CTRL", &ctrl_sock)
        .env("AVIO_SCREENS", "main")
        .env("AVIO_OUTPUT_APP_ID", &app_id);

    // Only trust the DRM-probed physical size on bare metal (no host session at all — the
    // compositor's own `wlr_backend_autocreate()` picks the real DRM/KMS backend in that case).
    // Once nested inside an existing desktop (WAYLAND_DISPLAY/DISPLAY set), the host places our
    // toplevel in ITS OWN logical/scaled coordinate space, and wlroots' wl_backend has no buffer
    // scale handling at all — requesting the panel's raw physical pixel count there just makes
    // the window come out `scale` times too big on any HiDPI desktop (confirmed against a KDE
    // session at 1.35x scale). So don't probe for a physical size there either; the window just
    // defaults to 1280x720 (and can be resized), rather than fighting the host compositor's
    // scaling.
    let host_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
    let nested = host_wayland_display.is_some() || std::env::var_os("DISPLAY").is_some();
    if !nested {
        if let Some((w, h)) = detect_output_size() {
            command.env("AVIO_OUTPUT_SIZE", format!("{w}x{h}"));
        }
    }

    // Lets `window::display_mode` (running inside the re-exec'd child, whose own
    // WAYLAND_DISPLAY now points at the *nested* compositor) still reach the host session to
    // query/switch its output mode via `wlr-randr` — see that module for why.
    if let Some(host_display) = &host_wayland_display {
        command.env("AVIO_HOST_WAYLAND_DISPLAY", host_display);
    }

    let spawn_result = command.spawn();

    let mut child = match spawn_result {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[compositor] failed to spawn {}: {e}", launcher.display());
            return false;
        }
    };

    // A dev watcher (cargo watch, ...) restarts on every source change by killing this (outer)
    // process directly, with no chance for our own cleanup code to run. Without a handler, avio-compositor
    // (and its own re-exec'd inner UI child) would just be orphaned rather than torn down, piling
    // up one more live instance on every single restart. Kill it ourselves first.
    let child_pid = child.id();
    std::thread::spawn(move || {
        let Ok(mut signals) =
            signal_hook::iterator::Signals::new([signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT])
        else {
            return;
        };
        if signals.forever().next().is_some() {
            unsafe {
                libc::kill(child_pid as libc::pid_t, libc::SIGTERM);
            }
        }
    });

    // Block here rather than returning right away: a dev runner watches this process's exit as
    // the signal that the app quit, and tears down the whole session (killing the compositor
    // mid-startup) the instant we return. Waiting for the compositor's own exit — which only
    // happens once its nested "-s" child (the re-exec'd avio instance) closes — makes this
    // process's lifetime match a normal single-process app's.
    match child.wait() {
        Ok(status) => {
            if !status.success() {
                eprintln!("[compositor] avio-compositor exited: {status}");
            }
        }
        Err(e) => eprintln!("[compositor] failed to wait on avio-compositor: {e}"),
    }
    true
}

#[cfg(not(target_os = "linux"))]
pub fn maybe_bootstrap() -> bool {
    false
}

/// Reads the first connected DRM output's preferred mode straight from sysfs (kernel-reported
/// modelines are listed highest-preferred-first), so we don't need a live X11/Wayland connection
/// — this runs before the outer process has one. Picks the first connected output found; fine
/// for a kiosk box with exactly one display, not meant to disambiguate a multi-monitor desktop.
#[cfg(target_os = "linux")]
fn detect_output_size() -> Option<(u32, u32)> {
    for entry in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let path = entry.path();
        let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
        if status.trim() != "connected" {
            continue;
        }
        let Ok(modes) = std::fs::read_to_string(path.join("modes")) else {
            continue;
        };
        let Some(first_mode) = modes.lines().next() else {
            continue;
        };
        let Some((w, h)) = first_mode.split_once('x') else {
            continue;
        };
        if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
            return Some((w, h));
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn launcher_path() -> Option<PathBuf> {
    // Packaged: the compositor/ tree sits next to the executable.
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.to_path_buf())) {
        let launcher = dir.join("compositor").join("avio-compositor");
        if launcher.exists() {
            return Some(launcher);
        }
    }

    // Dev: the compositor/ tree in the source tree (it's a whole bin+lib tree, not a single
    // relocatable sidecar next to our own exe like gst-host).
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("compositor")
        .join("avio-compositor");
    dev.exists().then_some(dev)
}
