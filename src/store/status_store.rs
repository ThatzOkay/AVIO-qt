use qtbridge::{QmlObject, qobject};

use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::Remote;
use crate::projection::driver::aa::commands::{aa_media_key, aa_resume, aa_send_touch};
use crate::projection::driver::aa::session_handle;
use crate::projection::driver::aa::stack::channels::input_channel::button_key;
use crate::projection::driver::aa::stack::channels::media_info_channel::{MediaPlaybackMetadata, MediaPlaybackState, MediaPlaybackStatus};
use crate::runtime;

static REMOTE: Remote = Remote::new();
// Size of the main window in pixels, as last reported by QML
static DISPLAY_SIZE: Mutex<Option<(u32, u32)>> = Mutex::new(None);

// A finger on the touch layer: QML's id for it, the small id Android Auto gets, and where it is
// (normalised 0..1)
struct Finger {
    qml_id: i32,
    aa_id: u32,
    x: f64,
    y: f64,
}

// Radio and Android Auto state. Read-only from QML; services call the free functions below
// and the values land on the Qt thread.
pub struct StatusStore {
    radio_fm: bool,
    // "" when there's no DAB hardware, else the backend ("WelleIo" or "WradioC100")
    radio_dab: String,
    // "disconnected", "connected" or "host-ui"
    aa_status: String,
    aa_device_name: String,
    // What the phone last reported as playing; reset when the session drops
    media_origin: Option<MediaOrigin>,
    media_status: MediaPlaybackStatus,
    media_metadata: MediaPlaybackMetadata,
    // Fingers currently down on the Android Auto touch layer, in the order they went down
    fingers: Vec<Finger>,
}

// Where the current media comes from; no origin means nothing is playing. Only Android Auto
// reports any so far.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum MediaOrigin {
    AndroidAuto,
    Bluetooth,
    Usb,
}

impl MediaOrigin {
    fn as_str(self) -> &'static str {
        match self {
            Self::AndroidAuto => "android-auto",
            Self::Bluetooth => "bluetooth",
            Self::Usb => "usb",
        }
    }
}

impl Default for StatusStore {
    fn default() -> Self {
        Self {
            radio_fm: false,
            radio_dab: String::new(),
            aa_status: "disconnected".to_string(),
            aa_device_name: String::new(),
            media_origin: None,
            media_status: MediaPlaybackStatus::default(),
            media_metadata: MediaPlaybackMetadata::default(),
            fingers: Vec::new(),
        }
    }
}

#[qobject(Singleton)]
impl StatusStore {
    qproperty!("radioFm", Read = radio_fm, Notify = radio_hardware_changed);
    qproperty!("radioDab", Read = radio_dab, Notify = radio_hardware_changed);
    qproperty!("radioAvailable", Read = radio_available, Notify = radio_hardware_changed);
    qproperty!("aaStatus", Read = aa_status, Notify = aa_changed);
    qproperty!("aaDeviceName", Read = aa_device_name, Notify = aa_changed);
    // "" when nothing is playing, else "android-auto", "bluetooth" or "usb"
    qproperty!("mediaOrigin", Read = media_origin, Notify = media_changed);
    // Whether any source is providing media, i.e. whether the media page has something to show
    qproperty!("mediaAvailable", Read = media_available, Notify = media_changed);
    // "unknown", "stopped", "playing" or "paused"
    qproperty!("mediaState", Read = media_state, Notify = media_changed);
    qproperty!("mediaSource", Read = media_source, Notify = media_changed);
    qproperty!("mediaPositionSeconds", Read = media_position_seconds, Notify = media_changed);
    qproperty!("mediaShuffle", Read = media_shuffle, Notify = media_changed);
    qproperty!("mediaRepeat", Read = media_repeat, Notify = media_changed);
    qproperty!("mediaRepeatOne", Read = media_repeat_one, Notify = media_changed);
    qproperty!("mediaSong", Read = media_song, Notify = media_changed);
    qproperty!("mediaArtist", Read = media_artist, Notify = media_changed);
    qproperty!("mediaAlbum", Read = media_album, Notify = media_changed);
    qproperty!("mediaPlaylist", Read = media_playlist, Notify = media_changed);
    qproperty!("mediaDurationSeconds", Read = media_duration_seconds, Notify = media_changed);
    qproperty!("mediaRating", Read = media_rating, Notify = media_changed);
    // A data: URL QML can use as an Image source, "" when there's no art
    qproperty!("mediaAlbumArt", Read = media_album_art, Notify = media_changed);

    #[qsignal]
    fn radio_hardware_changed(&mut self);
    #[qsignal]
    fn aa_changed(&mut self);
    #[qsignal]
    fn media_changed(&mut self);
    // Rust asks the main window to take focus back, see `request_focus`
    #[qsignal]
    fn focus_requested(&mut self);

    pub fn radio_fm(&self) -> bool {
        self.radio_fm
    }

    pub fn radio_dab(&self) -> String {
        self.radio_dab.clone()
    }

    pub fn radio_available(&self) -> bool {
        self.radio_fm || !self.radio_dab.is_empty()
    }

    pub fn aa_status(&self) -> String {
        self.aa_status.clone()
    }

    pub fn aa_device_name(&self) -> String {
        self.aa_device_name.clone()
    }

    pub fn media_available(&self) -> bool {
        self.media_origin.is_some()
    }

    pub fn media_origin(&self) -> String {
        self.media_origin.map(MediaOrigin::as_str).unwrap_or_default().to_string()
    }

    pub fn media_state(&self) -> String {
        match self.media_status.state {
            MediaPlaybackState::Unknown => "unknown",
            MediaPlaybackState::Stopped => "stopped",
            MediaPlaybackState::Playing => "playing",
            MediaPlaybackState::Paused => "paused",
        }
        .to_string()
    }

    pub fn media_source(&self) -> String {
        self.media_status.media_source.clone().unwrap_or_default()
    }

    pub fn media_position_seconds(&self) -> u32 {
        self.media_status.playback_seconds.unwrap_or_default()
    }

    pub fn media_shuffle(&self) -> bool {
        self.media_status.shuffle.unwrap_or_default()
    }

    pub fn media_repeat(&self) -> bool {
        self.media_status.repeat.unwrap_or_default()
    }

    pub fn media_repeat_one(&self) -> bool {
        self.media_status.repeat_one.unwrap_or_default()
    }

    pub fn media_song(&self) -> String {
        self.media_metadata.song.clone().unwrap_or_default()
    }

    pub fn media_artist(&self) -> String {
        self.media_metadata.artist.clone().unwrap_or_default()
    }

    pub fn media_album(&self) -> String {
        self.media_metadata.album.clone().unwrap_or_default()
    }

    pub fn media_playlist(&self) -> String {
        self.media_metadata.playlist.clone().unwrap_or_default()
    }

    pub fn media_duration_seconds(&self) -> u32 {
        self.media_metadata.duration_seconds.unwrap_or_default()
    }

    pub fn media_rating(&self) -> i32 {
        self.media_metadata.rating.unwrap_or_default()
    }

    pub fn media_album_art(&self) -> String {
        let Some(art) = self.media_metadata.album_art.as_deref().filter(|a| !a.is_empty()) else {
            return String::new();
        };
        let mime = if art.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" } else { "image/jpeg" };
        format!("data:{mime};base64,{}", STANDARD.encode(art))
    }

    // Call once from QML on startup so Rust can reach this object
    #[qslot]
    fn init(&self) {
        REMOTE.set(self.get_qml_method_invoker());
    }

    #[qslot]
    fn radio_hardware_set(&mut self, fm: bool, dab: String) {
        self.radio_fm = fm;
        self.radio_dab = dab;
        self.radio_hardware_changed();
    }

    #[qslot]
    fn focus_request(&mut self) {
        self.focus_requested();
    }

    // QML reports the main window's size whenever it changes
    #[qslot]
    fn set_display_size(&self, width: u32, height: u32) {
        *DISPLAY_SIZE.lock().expect("display size lock poisoned") = Some((width, height));
    }

    // The touch layer reports each finger as it goes down, moves and lifts (x/y normalised 0..1);
    // this turns that into the multi-touch gestures the phone expects.
    #[qslot]
    fn touch_down(&mut self, id: i32, x: f64, y: f64) {
        if self.fingers.iter().any(|f| f.qml_id == id) {
            return;
        }
        // The lowest id not in use, so they stay small like the phone's pointer ids
        let aa_id = (0..).find(|n| self.fingers.iter().all(|f| f.aa_id != *n)).unwrap_or(0);
        self.fingers.push(Finger { qml_id: id, aa_id, x, y });

        // A second (or third...) finger going down while others are active is a POINTER_DOWN on the
        // existing gesture, not a fresh DOWN - a plain DOWN would reset the phone's touch state
        // machine instead of adding a pointer to it.
        let phase = if self.fingers.len() == 1 { "down" } else { "pointerdown" };
        self.send_touch(phase, self.fingers.len() - 1);
    }

    #[qslot]
    fn touch_move(&mut self, id: i32, x: f64, y: f64) {
        let Some(finger) = self.fingers.iter_mut().find(|f| f.qml_id == id) else {
            return;
        };
        finger.x = x;
        finger.y = y;
        self.send_touch("move", 0);
    }

    #[qslot]
    fn touch_up(&mut self, id: i32, x: f64, y: f64) {
        let Some(index) = self.fingers.iter().position(|f| f.qml_id == id) else {
            return;
        };
        self.fingers[index].x = x;
        self.fingers[index].y = y;
        // The phone needs the pointer set as it stood just before the lift, with the action index
        // pointing at the one that went up (POINTER_UP), or a plain UP once every finger is off
        let phase = if self.fingers.len() == 1 { "up" } else { "pointerup" };
        self.send_touch(phase, index);
        self.fingers.remove(index);
    }

    // The phone kicked the display back to its own UI ("host-ui"); take projection back
    #[qslot]
    fn resume(&self) {
        runtime::spawn(async {
            if let Err(e) = aa_resume(&session_handle::shared()).await {
                eprintln!("aa resume: {e}");
            }
        });
    }

    #[qslot]
    fn media_play_pause(&self) {
        send_media_key(button_key::MEDIA_PLAY_PAUSE);
    }

    #[qslot]
    fn media_next(&self) {
        send_media_key(button_key::MEDIA_NEXT);
    }

    #[qslot]
    fn media_previous(&self) {
        send_media_key(button_key::MEDIA_PREV);
    }

    fn send_touch(&self, phase: &'static str, action_index: usize) {
        let touches = self.fingers.iter().map(|f| (f.x, f.y, f.aa_id)).collect();
        runtime::spawn(async move {
            let handle = session_handle::shared();
            if let Err(e) = aa_send_touch(&handle, touches, phase.to_string(), action_index as u32).await {
                eprintln!("aa touch: {e}");
            }
        });
    }

    #[qslot]
    fn aa_status_set(&mut self, status: String, device_name: String) {
        // Nothing is playing once the phone is gone
        let dropped = status == "disconnected";
        self.aa_status = status;
        self.aa_device_name = device_name;
        self.aa_changed();
        if dropped {
            self.media_origin = None;
            self.media_status = MediaPlaybackStatus::default();
            self.media_metadata = MediaPlaybackMetadata::default();
            self.media_changed();
        }
    }

    #[qslot]
    fn aa_media_status_set(&mut self, status: String) {
        match serde_json::from_str(&status) {
            Ok(status) => {
                self.media_origin = Some(MediaOrigin::AndroidAuto);
                // Updates are partial (a position-only one is common), so merge, don't replace
                self.media_status.merge(status);
            }
            Err(e) => return eprintln!("aa media status: {e}"),
        }
        self.media_changed();
    }

    #[qslot]
    fn aa_media_metadata_set(&mut self, metadata: String) {
        match serde_json::from_str(&metadata) {
            Ok(metadata) => {
                self.media_origin = Some(MediaOrigin::AndroidAuto);
                self.media_metadata = metadata;
            }
            Err(e) => return eprintln!("aa media metadata: {e}"),
        }
        self.media_changed();
    }
}

fn send_media_key(key: u32) {
    runtime::spawn(async move {
        if let Err(e) = aa_media_key(&session_handle::shared(), key).await {
            eprintln!("aa media key: {e}");
        }
    });
}

// Safe from any thread. Dropped if QML hasn't called StatusStore.init() yet.
pub fn set_radio_hardware(fm: bool, dab: Option<&str>) {
    let dab = dab.unwrap_or_default().to_string();
    REMOTE.with(|invoker| {
        qtbridge::invoke_method!(invoker, "radio_hardware_set", fm, dab);
    });
}

pub fn set_aa_status(status: &str, device_name: Option<&str>) {
    let status = status.to_string();
    let device_name = device_name.unwrap_or_default().to_string();
    REMOTE.with(|invoker| {
        qtbridge::invoke_method!(invoker, "aa_status_set", status, device_name);
    });
}

pub fn set_aa_media_status(status: MediaPlaybackStatus) {
    let status = serde_json::to_string(&status).expect("media status serialises");
    REMOTE.with(|invoker| {
        qtbridge::invoke_method!(invoker, "aa_media_status_set", status);
    });
}

pub fn set_aa_media_metadata(metadata: MediaPlaybackMetadata) {
    let metadata = serde_json::to_string(&metadata).expect("media metadata serialises");
    REMOTE.with(|invoker| {
        qtbridge::invoke_method!(invoker, "aa_media_metadata_set", metadata);
    });
}

// The main window's size, once QML has reported it
pub fn display_size() -> Option<(u32, u32)> {
    *DISPLAY_SIZE.lock().expect("display size lock poisoned")
}

// Asks QML to re-activate the main window, e.g. after the video toplevel stole focus
pub fn request_focus() {
    REMOTE.with(|invoker| {
        qtbridge::invoke_method!(invoker, "focus_request");
    });
}
