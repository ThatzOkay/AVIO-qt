//! Media playback status channel handler (`CH.MEDIA_INFO` = 13).
//!
//! Phone -> HU only (we don't drive playback metadata, we just receive it).
//!
//! Wire protocol (aap_protobuf.service.mediaplayback):
//!   MEDIA_PLAYBACK_STATUS    = 32769  -> MediaPlaybackStatus { state, media_source, position, shuffle/repeat }
//!   MEDIA_PLAYBACK_INPUT     = 32770  -> playback control (HU -> Phone, not implemented here)
//!   MEDIA_PLAYBACK_METADATA  = 32771  -> MediaPlaybackMetadata { song, artist, album, album_art, ... }
//!
//! Channel-open response is handled by `Session`'s generic CHANNEL_OPEN_REQUEST path; nothing
//! channel-specific to do there.

use serde::{Deserialize, Serialize};

use super::proto_enc::{decode_fields, decode_varint_value};

mod media_msg {
    pub const MEDIA_PLAYBACK_STATUS: u16 = 0x8001; // = 32769
    pub const MEDIA_PLAYBACK_INPUT: u16 = 0x8002; // = 32770
    pub const MEDIA_PLAYBACK_METADATA: u16 = 0x8003; // = 32771
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaPlaybackMetadata {
    pub song: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub playlist: Option<String>,
    pub duration_seconds: Option<u32>,
    pub rating: Option<i32>,
    /// Raw album-art bytes (typically JPEG/PNG) — pass through to the UI as-is.
    pub album_art: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MediaPlaybackState {
    #[default]
    Unknown,
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaPlaybackStatus {
    pub state: MediaPlaybackState,
    pub media_source: Option<String>,
    pub playback_seconds: Option<u32>,
    pub shuffle: Option<bool>,
    pub repeat: Option<bool>,
    pub repeat_one: Option<bool>,
}

#[derive(Debug)]
pub enum MediaInfoEvent {
    Metadata(MediaPlaybackMetadata),
    Status(MediaPlaybackStatus),
    None,
}

pub fn handle_message(msg_id: u16, payload: &[u8]) -> MediaInfoEvent {
    match msg_id {
        media_msg::MEDIA_PLAYBACK_METADATA => MediaInfoEvent::Metadata(decode_metadata(payload)),
        media_msg::MEDIA_PLAYBACK_STATUS => MediaInfoEvent::Status(decode_status(payload)),
        // HU->Phone direction in aasdk; if the phone ever echoes one back to us we just ignore
        // it — playback control is one-way from us (not implemented here).
        media_msg::MEDIA_PLAYBACK_INPUT => MediaInfoEvent::None,
        _ => MediaInfoEvent::None,
    }
}

fn decode_metadata(payload: &[u8]) -> MediaPlaybackMetadata {
    let mut out = MediaPlaybackMetadata::default();
    for f in decode_fields(payload) {
        match f.field {
            1 => out.song = Some(String::from_utf8_lossy(&f.bytes).into_owned()),
            2 => out.artist = Some(String::from_utf8_lossy(&f.bytes).into_owned()),
            3 => out.album = Some(String::from_utf8_lossy(&f.bytes).into_owned()),
            4 => out.album_art = Some(f.bytes),
            5 => out.playlist = Some(String::from_utf8_lossy(&f.bytes).into_owned()),
            6 => out.duration_seconds = Some(decode_varint_value(&f.bytes)),
            7 => out.rating = Some(decode_varint_value(&f.bytes) as i32),
            _ => {}
        }
    }
    out
}

fn decode_status(payload: &[u8]) -> MediaPlaybackStatus {
    let mut out = MediaPlaybackStatus::default();
    for f in decode_fields(payload) {
        match f.field {
            1 => {
                out.state = match decode_varint_value(&f.bytes) {
                    1 => MediaPlaybackState::Stopped,
                    2 => MediaPlaybackState::Playing,
                    3 => MediaPlaybackState::Paused,
                    _ => MediaPlaybackState::Unknown,
                };
            }
            2 => out.media_source = Some(String::from_utf8_lossy(&f.bytes).into_owned()),
            3 => out.playback_seconds = Some(decode_varint_value(&f.bytes)),
            4 => out.shuffle = Some(decode_varint_value(&f.bytes) != 0),
            5 => out.repeat = Some(decode_varint_value(&f.bytes) != 0),
            6 => out.repeat_one = Some(decode_varint_value(&f.bytes) != 0),
            _ => {}
        }
    }
    out
}
