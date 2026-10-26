fn main() {
    // Tell cargo to tell rustc to link the system bzip2
    // shared library.
    println!("cargo:rustc-link-lib=bz2");

    let mut build = cxx_build::bridge("src/lib.rs");
    build
        .file("src/shim.cpp")
        .include("src")
        .include("vendor/welle.io/src")
        .include("vendor/welle.io/src/backend")
        .include("vendor/welle.io/src/various")
        .include("vendor/welle.io/src/input")
        .std("gnu++17");

    // Homebrew's fftw formula installs fftw3.h/libfftw3f into the Homebrew
    // prefix's include/lib dirs, but those aren't on clang's default search
    // path (unlike Linux's apt-installed libfftw3-dev, which lands in a
    // standard system location). Ask brew directly rather than hardcoding
    // /opt/homebrew vs /usr/local, since that differs between Apple Silicon
    // and Intel.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        if let Ok(output) = std::process::Command::new("brew")
            .args(["--prefix", "fftw"])
            .output()
        {
            if output.status.success() {
                let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
                build.include(format!("{prefix}/include"));
                println!("cargo:rustc-link-search=native={prefix}/lib");
            }
        }
    }

    // fftw3 has no package for the default C:\mingw64 toolchain, so CI
    // installs it via MSYS2 instead and points us at its headers here.
    // Deliberately no matching -L/link-search: nothing links against fftw3
    // yet (only its header is used), and adding MSYS2's lib dir to the
    // search path pulls in its libpthread.a/CRT import libs ahead of
    // C:\mingw64's own for unrelated symbols too (undefined reference to
    // `_assert` etc.) - the same cross-toolchain mixing bug as compiling
    // with a different MinGW than what links, just via -L instead of CC/CXX.
    //
    // -idirafter rather than -I (build.include): a plain -I searches MSYS2's
    // include dir *before* C:\mingw64's system headers, so <wchar.h> resolves
    // to MSYS2's copy, whose corecrt_wstdlib.h defines wcstod/wcstof again
    // after C:\mingw64's stdlib.h already did ("redefinition of 'double
    // wcstod'"). -idirafter makes it a last resort, so only headers that
    // C:\mingw64 doesn't have at all (fftw3.h) come from MSYS2.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if let Ok(root) = std::env::var("MSYS2_MINGW64_ROOT") {
            build.flag("-idirafter").flag(format!("{root}/include"));
        }
    }

    build.compile("welle-io-sys-bridge");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=src/shim.h");
}
