fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();

    // GStreamer dev headers/libs are located via pkg-config on every platform (the official
    // Windows/macOS GStreamer "devel" packages ship .pc files too; point PKG_CONFIG_PATH at
    // <root>/lib/pkgconfig if pkg-config can't find them on its own).
    let gst_libs = ["gstreamer-1.0", "gstreamer-app-1.0", "gstreamer-video-1.0", "gstreamer-base-1.0"];
    let mut build = cxx_build::bridge("src/lib.rs");

    for lib in gst_libs {
        let lib_info = pkg_config::Config::new()
            .atleast_version("1.14")
            .probe(lib)
            .unwrap_or_else(|e| panic!("pkg-config could not find {lib}: {e}"));
        for path in lib_info.include_paths {
            build.include(path);
        }
    }

    build
        .file("src/native/gst_video.cc")
        .flag_if_supported("-std=c++17")
        .warnings(false);

    match target_os.as_str() {
        "macos" => {
            build.file("src/native/gst_video_mac.mm");
            build.flag("-x").flag("objective-c++").flag("-fobjc-arc");
            println!("cargo:rustc-link-lib=framework=Cocoa");
            println!("cargo:rustc-link-lib=framework=QuartzCore");
        }
        "windows" => {
            build.file("src/native/gst_video_win.cc");
            build.define("NOMINMAX", None);
            println!("cargo:rustc-link-lib=comctl32");
        }
        _ => {}
    }

    build.compile("gst-video-native");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/native/gst_video.h");
    println!("cargo:rerun-if-changed=src/native/gst_video.cc");
    println!("cargo:rerun-if-changed=src/native/gst_video_mac.mm");
    println!("cargo:rerun-if-changed=src/native/gst_video_win.cc");
}
