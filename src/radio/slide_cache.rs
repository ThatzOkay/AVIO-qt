// The newest slideshow picture (DAB MOT) of each station, kept on disk under the cache dir so a
// station shows its last picture right after tuning, before the broadcast sends one again.
//
// One file per station, named <service id>-<content hash>.<ext>: a new picture gets a new path, which
// is what makes QML's Image reload it, and the previous file of that station is removed.
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// Stations a head unit has seen are in the tens, this only guards against a runaway
const MAX_FILES: usize = 256;

static INDEX: Mutex<Option<HashMap<u32, PathBuf>>> = Mutex::new(None);

fn dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join(env!("CARGO_PKG_NAME")).join("slides"))
}

// Only what Qt shows and the slideshow commonly carries: JPEG and PNG
fn extension(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        _ => None,
    }
}

fn service_id(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    u32::from_str_radix(name.split('-').next()?, 16).ok()
}

// Reads the directory once; later lookups and stores keep the map current
fn with_index<T>(f: impl FnOnce(&mut HashMap<u32, PathBuf>) -> T) -> T {
    let mut guard = INDEX.lock().unwrap();
    let index = guard.get_or_insert_with(|| {
        let mut index = HashMap::new();
        if let Some(entries) = dir().and_then(|dir| std::fs::read_dir(dir).ok()) {
            for path in entries.flatten().map(|entry| entry.path()) {
                if let Some(id) = service_id(&path) {
                    index.insert(id, path);
                }
            }
        }
        index
    });
    f(index)
}

/// The cached picture of a station, if it has one.
pub fn latest(service_id: u32) -> Option<PathBuf> {
    with_index(|index| index.get(&service_id).cloned())
}

/// Every station that has a cached picture, for the station list.
pub fn all() -> HashMap<u32, PathBuf> {
    with_index(|index| index.clone())
}

/// Saves a slideshow picture as the station's newest and returns where it is. None for a picture
/// that isn't JPEG or PNG, or when the cache dir can't be written.
pub fn store(service_id: u32, mime_type: &str, data: &[u8]) -> Option<PathBuf> {
    let extension = extension(mime_type)?;
    if data.is_empty() {
        return None;
    }
    let dir = dir()?;

    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    let path = dir.join(format!("{service_id:08x}-{:016x}.{extension}", hasher.finish()));

    // The same picture is rebroadcast every few seconds
    if with_index(|index| index.get(&service_id) == Some(&path)) && path.exists() {
        return Some(path);
    }

    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("slides: creating {}: {e}", dir.display());
        return None;
    }
    // Written aside and renamed, so a power cut never leaves half a picture that would then be shown
    let tmp = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, &path)) {
        eprintln!("slides: writing {}: {e}", path.display());
        return None;
    }

    with_index(|index| {
        if let Some(old) = index.insert(service_id, path.clone()) {
            if old != path {
                let _ = std::fs::remove_file(old);
            }
        }
        if index.len() > MAX_FILES {
            prune(index);
        }
    });
    Some(path)
}

// Drops the least recently written pictures until the cap holds again
fn prune(index: &mut HashMap<u32, PathBuf>) {
    let mut by_age: Vec<_> = index
        .iter()
        .map(|(id, path)| {
            let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
            (modified, *id)
        })
        .collect();
    by_age.sort();
    let excess = index.len() - MAX_FILES;
    for (_, id) in by_age.into_iter().take(excess) {
        if let Some(path) = index.remove(&id) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// `file://` URL for QML's Image.source.
pub fn file_url(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The one test that touches the process-wide cache dir and index
    #[test]
    fn keeps_only_the_newest_picture_per_station() {
        let dir = std::env::temp_dir().join(format!("avio-slides-test-{}", std::process::id()));
        // SAFETY: no other test in this binary reads XDG_CACHE_HOME
        unsafe { std::env::set_var("XDG_CACHE_HOME", &dir) };

        assert_eq!(latest(7), None);
        assert_eq!(store(7, "image/gif", b"gif"), None);
        assert_eq!(store(7, "image/png", b""), None);

        let first = store(7, "image/png", b"first").unwrap();
        assert_eq!(first.extension().unwrap(), "png");
        assert_eq!(store(7, "image/png", b"first"), Some(first.clone()));

        let second = store(7, "image/jpeg", b"second").unwrap();
        assert_ne!(first, second);
        assert!(!first.exists());
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
        assert_eq!(latest(7), Some(second.clone()));
        assert_eq!(all().get(&7), Some(&second));

        assert!(file_url(&second).starts_with("file:///"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
