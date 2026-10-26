use std::path::PathBuf;

// Bundled files (gstreamer, ...). A shipped build keeps them in `assets/` next to the executable;
// during development they're read straight from the source tree.
pub fn assets_dir() -> PathBuf {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("assets")));
    match bundled {
        Some(dir) if dir.is_dir() => dir,
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("assets"),
    }
}
