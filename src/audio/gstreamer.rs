use std::path::{Path, PathBuf};


pub enum VideoCodec {
    H264,
    H265,
}

fn platform_dir() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("macos", "x86_64") => Some("macos-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("windows", "x86_64") => Some("windows-x64"),
        _ => None,
    }
}

// The bundled GStreamer for this platform, if there is one. Desktop builds ship it; an embedded
// image (Buildroot, ...) uses the system GStreamer instead, see `launch_command`.
pub fn resolve_gstreamer_root() -> Option<PathBuf> {
    let dir = platform_dir()?;
    Some(crate::paths::assets_dir().join("gstreamer").join(dir)).filter(|root| root.is_dir())
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

// The GStreamer tool `name` ("gst-launch-1.0", ...) and the environment to run it with: the
// bundled one when there is one, else the system's, found on PATH with the ambient environment.
pub fn launch_command(name: &str) -> (PathBuf, Vec<(String, String)>) {
    match resolve_gstreamer_root() {
        Some(root) => (root.join("bin").join(executable_name(name)), gst_env(&root)),
        None => (PathBuf::from(executable_name(name)), Vec::new()),
    }
}

// Same as `launch_command`, with the environment for device enumeration (see `gst_env_for_enum`).
pub fn enum_command(name: &str) -> (PathBuf, Vec<(String, String)>) {
    match resolve_gstreamer_root() {
        Some(root) => (root.join("bin").join(executable_name(name)), gst_env_for_enum(&root)),
        None => (PathBuf::from(executable_name(name)), Vec::new()),
    }
}

/// Environment for spawning gst-device-monitor during device enumeration.
/// The bundled binary locates its own libs and plugins via RPATH + baked-in paths, so
/// we must not override GST_PLUGIN_PATH / GST_PLUGIN_SYSTEM_PATH / LD_LIBRARY_PATH here —
/// those overrides break the device providers (PipeWire/PulseAudio) that need system libs.
/// On Windows the binary still needs its bin dir on PATH to find DLLs.
pub fn gst_env_for_enum(gst_root: &Path) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars().collect();

    let scanner = if std::env::consts::OS == "windows" {
        gst_root
            .join("libexec")
            .join("gstreamer-1.0")
            .join("gst-plugin-scanner.exe")
    } else {
        gst_root
            .join("libexec")
            .join("gstreamer-1.0")
            .join("gst-plugin-scanner")
    };
    if scanner.exists() {
        env.push((
            "GST_PLUGIN_SCANNER".to_owned(),
            scanner.to_str().unwrap_or_default().to_owned(),
        ));
    }

    if std::env::consts::OS == "windows" {
        env.push((
            "PATH".to_owned(),
            gst_root.join("bin").to_str().unwrap_or_default().to_owned(),
        ));
    }

    env
}

pub fn gst_env(gst_root: &Path) -> Vec<(String, String)> {
    let mut env = vec![];
    let plugin_path = gst_root.join("lib").join("gstreamer-1.0");
    let plugin_scanner = if std::env::consts::OS == "windows" {
        gst_root
            .join("libexec")
            .join("gstreamer-1.0")
            .join("gst-plugin-scanner.exe")
    } else {
        gst_root
            .join("libexec")
            .join("gstreamer-1.0")
            .join("gst-plugin-scanner")
    };

    let current_env = std::env::vars().collect::<Vec<(String, String)>>();

    env.extend(current_env);

    let registry = std::env::temp_dir().join("avio-gstreamer-registry.bin");

    // Inside an AppImage there's no meaningful "system" GStreamer to fall back to - the host may
    // have an incompatible or absent install, so isolate to just the bundled plugins by blanking
    // the system path. Outside one (a raw dev/debug binary - devcontainer, `cargo run`, etc. -
    // where a bundled distro may be partial or absent), leave it alone so the host's own
    // gstreamerN.0-* packages still fill in whatever the bundle doesn't cover.
    let in_appimage = std::env::var_os("APPIMAGE").is_some();

    // GStreamer prefers the "_1_0"-suffixed variant of each var over the plain name when both
    // are set - AppImage's linuxdeploy-generated AppRun sets the _1_0 ones pointing elsewhere,
    // silently winning over the plain overrides below unless we also override the versioned ones.
    for suffix in ["", "_1_0"] {
        if in_appimage {
            env.push((format!("GST_PLUGIN_SYSTEM_PATH{suffix}"), "".to_owned()));
        }
        env.push((
            format!("GST_PLUGIN_PATH{suffix}"),
            plugin_path.to_str().unwrap_or_default().to_owned(),
        ));
        env.push((
            format!("GST_PLUGIN_SCANNER{suffix}"),
            plugin_scanner.to_str().unwrap_or_default().to_owned(),
        ));
    }
    env.push((
        "GST_REGISTRY".to_owned(),
        registry.to_str().unwrap_or_default().to_owned(),
    ));

    if std::env::consts::OS == "macos" {
        env.push((
            "DYLD_LIBRARY_PATH".to_owned(),
            gst_root.join("lib").to_str().unwrap_or_default().to_owned(),
        ));
    } else if std::env::consts::OS == "linux" {
        env.push((
            "LD_LIBRARY_PATH".to_owned(),
            gst_root.join("lib").to_str().unwrap_or_default().to_owned(),
        ));
    } else if std::env::consts::OS == "windows" {
        env.push((
            "PATH".to_owned(),
            gst_root.join("bin").to_str().unwrap_or_default().to_owned(),
        ));
    }

    env
}

pub fn audio_sink_element() -> &'static str {
    if std::env::consts::OS == "macos" {
        "osxaudiosink"
    } else if std::env::consts::OS == "linux" {
        "pulsesink"
    } else if std::env::consts::OS == "windows" {
        "wasapisink"
    } else {
        "autoaudiosink"
    }
}

pub fn audio_source_element() -> &'static str {
    if std::env::consts::OS == "macos" {
        "osxaudiosrc"
    } else if std::env::consts::OS == "linux" {
        "pulsesrc"
    } else if std::env::consts::OS == "windows" {
        "wasapisrc"
    } else {
        "autoaudiosrc"
    }
}

pub fn audio_device_prop() -> &'static str {
    if std::env::consts::OS == "macos" {
        "unique-id"
    } else if std::env::consts::OS == "linux" {
        "device"
    } else if std::env::consts::OS == "windows" {
        "device-name"
    } else {
        "device"
    }
}

pub fn video_parse_element(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "h264parse",
        VideoCodec::H265 => "h265parse",
    }
}

pub fn video_decoder_element(codec: VideoCodec) -> &'static str {
    if std::env::consts::OS == "macos" {
        "vtdec"
    } else if std::env::consts::OS == "windows" {
        match codec {
            VideoCodec::H264 => "d3d11h264dec",
            VideoCodec::H265 => "d3d11h265dec",
        }
    } else {
        match codec {
            VideoCodec::H264 => "v4l2slh264dec",
            VideoCodec::H265 => "v4l2slh265dec",
        }
    }
}

pub fn video_sink_element() -> &'static str {
    if std::env::consts::OS == "windows" {
        "d3d11videosink"
    } else {
        "glimagesink"
    }
}
