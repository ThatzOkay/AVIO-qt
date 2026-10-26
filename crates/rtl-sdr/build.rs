use std::env;
use std::path::Path;

fn main() {
    // rtl_sdr_detection.rs and fm_radio.rs link `rtlsdr` directly
    // (`#[link(name = "rtlsdr")]`). There's no vcpkg port and upstream ships no
    // Windows binaries, so a prebuilt DLL + a generated import lib are vendored
    // under vendor/windows-x64 (see the README there).
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let vendor_dir = Path::new(&manifest_dir).join("vendor/windows-x64");
        println!("cargo:rustc-link-search=native={}", vendor_dir.display());
    }

    // Homebrew's librtlsdr formula links its dylib into the Homebrew prefix's lib/
    // dir, but that dir isn't reliably on the default linker search path (unlike
    // Linux's apt-installed librtlsdr-dev, which lands in a standard system
    // location). Ask brew directly rather than hardcoding /opt/homebrew vs
    // /usr/local, since that differs between Apple Silicon and Intel.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        if let Ok(output) = std::process::Command::new("brew")
            .args(["--prefix", "librtlsdr"])
            .output()
        {
            if output.status.success() {
                let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("cargo:rustc-link-search=native={prefix}/lib");
            }
        }
    }
}
