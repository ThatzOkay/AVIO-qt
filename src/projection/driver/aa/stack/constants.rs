//! Android Auto wire-protocol constants.

// ── Frame flags (byte 1 of the 4-byte frame header) ─────────────────────────
pub mod frame_flags {
    pub const PLAINTEXT: u8 = 0x03; // version negotiation + SSL handshake
    pub const ENC_SIGNAL: u8 = 0x0b; // encrypted single-frame signaling
    pub const ENC_CONTROL: u8 = 0x0f; // encrypted channel-lifecycle messages
    pub const ENC_FIRST_FRAG: u8 = 0x08; // encrypted first fragment
    pub const ENC_CONT_FRAG: u8 = 0x0a; // encrypted continuation/last fragment
}

// ── Control-channel message IDs (channel 0) ──────────────────────────────────
// Source: oaa/control/ControlMessageIdsEnum.proto
pub mod ctrl_msg {
    pub const VERSION_REQUEST: u16 = 0x0001;
    pub const VERSION_RESPONSE: u16 = 0x0002;
    pub const SSL_HANDSHAKE: u16 = 0x0003;
    pub const AUTH_COMPLETE: u16 = 0x0004;
    pub const SERVICE_DISCOVERY_REQUEST: u16 = 0x0005; // phone -> HU (despite the name)
    pub const SERVICE_DISCOVERY_RESPONSE: u16 = 0x0006; // HU -> phone, sent in reply to REQUEST
    pub const CHANNEL_OPEN_REQUEST: u16 = 0x0007; // phone -> HU (phone initiates each channel)
    pub const CHANNEL_OPEN_RESPONSE: u16 = 0x0008; // HU -> phone
    pub const CHANNEL_CLOSE_NOTIFICATION: u16 = 0x0009;
    pub const PING_REQUEST: u16 = 0x000b;
    pub const PING_RESPONSE: u16 = 0x000c;
    pub const NAVIGATION_FOCUS_REQUEST: u16 = 0x000d;
    pub const NAVIGATION_FOCUS_RESPONSE: u16 = 0x000e;
    pub const SHUTDOWN_REQUEST: u16 = 0x000f;
    pub const SHUTDOWN_RESPONSE: u16 = 0x0010;
    pub const VOICE_SESSION_NOTIFICATION: u16 = 0x0011; // phone -> HU (1=START, 2=END)
    pub const AUDIO_FOCUS_REQUEST: u16 = 0x0012;
    pub const AUDIO_FOCUS_RESPONSE: u16 = 0x0013;
    pub const BINDING_REQUEST: u16 = 0x0019; // phone -> HU (scan codes)
    pub const BINDING_RESPONSE: u16 = 0x001a; // HU -> phone
}

// ── AV-channel message IDs (channels 2-9) ────────────────────────────────────
// Source: oaa/av/AVChannelMessageIdsEnum.proto
pub mod av_msg {
    // Raw media data (low IDs, no high bit)
    pub const AV_MEDIA_WITH_TIMESTAMP: u16 = 0x0000;
    pub const AV_MEDIA_INDICATION: u16 = 0x0001; // H.264 / PCM frames
                                                 // Signaling (high bit set = Specific message type)
    pub const SETUP_REQUEST: u16 = 0x8000;
    pub const START_INDICATION: u16 = 0x8001;
    pub const STOP_INDICATION: u16 = 0x8002;
    pub const SETUP_RESPONSE: u16 = 0x8003;
    pub const AV_MEDIA_ACK: u16 = 0x8004;
    pub const AV_INPUT_OPEN_REQUEST: u16 = 0x8005;
    pub const AV_INPUT_OPEN_RESPONSE: u16 = 0x8006;
    pub const VIDEO_FOCUS_REQUEST: u16 = 0x8007;
    pub const VIDEO_FOCUS_INDICATION: u16 = 0x8008;
}

// ── Radio-channel message IDs (channel 11) ───────────────────────────────────
// Source: aap_protobuf/service/radio/RadioMessageId.proto
pub mod radio_msg {
    pub const ACTIVE_RADIO_NOTIFICATION: u16 = 32769;
    pub const SELECT_ACTIVE_RADIO_REQUEST: u16 = 32770;
    pub const STEP_CHANNEL_REQUEST: u16 = 32771;
    pub const STEP_CHANNEL_RESPONSE: u16 = 32772;
    pub const SEEK_STATION_REQUEST: u16 = 32773;
    pub const SEEK_STATION_RESPONSE: u16 = 32774;
    pub const SCAN_STATIONS_REQUEST: u16 = 32775;
    pub const SCAN_STATIONS_RESPONSE: u16 = 32776;
    pub const TUNE_TO_STATION_REQUEST: u16 = 32777;
    pub const TUNE_TO_STATION_RESPONSE: u16 = 32778;
    pub const GET_PROGRAM_LIST_REQUEST: u16 = 32779;
    pub const GET_PROGRAM_LIST_RESPONSE: u16 = 32780;
    pub const STATION_PRESETS_NOTIFICATION: u16 = 32781;
    pub const CANCEL_OPERATIONS_REQUEST: u16 = 32782;
    pub const CANCEL_OPERATIONS_RESPONSE: u16 = 32783;
    pub const CONFIGURE_CHANNEL_SPACING_REQUEST: u16 = 32784;
    pub const CONFIGURE_CHANNEL_SPACING_RESPONSE: u16 = 32785;
    pub const RADIO_STATION_INFO_NOTIFICATION: u16 = 32786;
    pub const MUTE_RADIO_REQUEST: u16 = 32787;
    pub const MUTE_RADIO_RESPONSE: u16 = 32788;
    pub const GET_TRAFFIC_UPDATE_REQUEST: u16 = 32789;
    pub const GET_TRAFFIC_UPDATE_RESPONSE: u16 = 32790;
    pub const RADIO_SOURCE_REQUEST: u16 = 32791;
    pub const RADIO_SOURCE_RESPONSE: u16 = 32792;
    pub const STATE_NOTIFICATION: u16 = 32793;
}

// ── Channel IDs (GAL service types) ──────────────────────────────────────────
// Source: aasdk messenger::ChannelId enum.
pub mod ch {
    pub const CONTROL: u8 = 0;
    pub const SENSOR: u8 = 1; // driving status, GPS, night mode
                              // (2 = MEDIA_SINK group header)
    pub const VIDEO: u8 = 3; // main display H.264/H.265   (MEDIA_SINK_VIDEO)
    pub const MEDIA_AUDIO: u8 = 4; // music / podcast PCM  (MEDIA_SINK_MEDIA_AUDIO)
    pub const SPEECH_AUDIO: u8 = 5; // navigation prompts  (MEDIA_SINK_GUIDANCE_AUDIO)
    pub const SYSTEM_AUDIO: u8 = 6; // system sounds       (MEDIA_SINK_SYSTEM_AUDIO)
                                    // (7 = MEDIA_SINK_TELEPHONY_AUDIO)
    pub const INPUT: u8 = 8; // touch + keycodes (INPUT_SOURCE)
    pub const MIC_INPUT: u8 = 9; // microphone from phone  (MEDIA_SOURCE_MICROPHONE)
    pub const BLUETOOTH: u8 = 10;
    pub const RADIO: u8 = 11; // FM/AM radio (RADIO_SOURCE)
    pub const NAVIGATION: u8 = 12; // NAVIGATION_STATUS
    pub const MEDIA_INFO: u8 = 13; // MEDIA_PLAYBACK_STATUS
    pub const PHONE_STATUS: u8 = 14;
    // (15 MEDIA_BROWSER, 16 VENDOR_EXTENSION, 17 GENERIC_NOTIFICATION)
    pub const WIFI: u8 = 18; // WIFI_PROJECTION
    pub const CLUSTER_VIDEO: u8 = 19; // secondary display sink (display_type=CLUSTER)
    pub const CLUSTER_INPUT: u8 = 20; // secondary display input stub (display_id=1, non-interactive)
}

// ── Version negotiation ───────────────────────────────────────────────────────
pub mod version {
    pub const MAJOR: u16 = 1;
    pub const MINOR: u16 = 7;
    pub const STATUS_MATCH: u16 = 0x0000;
    pub const STATUS_MISMATCH: u16 = 0xffff;
}

pub const TCP_PORT: u16 = 5277;
pub const STATUS_OK: i32 = 0;

pub mod video_resolution {
    pub const R800X480: i32 = 1;
    pub const R1280X720: i32 = 2;
    pub const R1920X1080: i32 = 3;
}

pub mod video_fps {
    pub const FPS60: i32 = 1;
    pub const FPS30: i32 = 2;
}

pub mod media_codec {
    pub const AUDIO_PCM: i32 = 1;
    pub const AUDIO_AAC_LC: i32 = 2;
    pub const VIDEO_H264_BP: i32 = 3;
    pub const VIDEO_VP9: i32 = 5;
    pub const VIDEO_AV1: i32 = 6;
    pub const VIDEO_H265: i32 = 7;
}

pub mod av_stream_type {
    pub const AUDIO: i32 = 1;
    pub const VIDEO: i32 = 3;
}

// Source: oaa.proto.enums.DisplayType — proto3 enum (0-based).
pub mod display_type {
    pub const MAIN: i32 = 0;
    pub const CLUSTER: i32 = 1;
    pub const AUXILIARY: i32 = 2;
}

pub mod sensor_type {
    pub const DRIVING_STATUS: i32 = 13;
    pub const NIGHT_DATA: i32 = 10;
    pub const PARKING_BRAKE: i32 = 7;
    pub const GPS_LOCATION: i32 = 1;
    pub const CAR_SPEED: i32 = 3;
    pub const RPM: i32 = 4;
}

pub mod audio_type {
    pub const SPEECH: i32 = 1;
    pub const SYSTEM: i32 = 2;
    pub const MEDIA: i32 = 3;
}

pub mod bt_pairing_method {
    pub const NUMERIC_COMPARISON: i32 = 2;
    pub const PIN: i32 = 4;
}

pub mod color_scheme {
    pub const BASIC: i32 = 0;
    pub const MATERIAL_YOU_V2: i32 = 2;
    pub const MATERIAL_YOU_V3: i32 = 3;
}

pub mod av_setup_status {
    pub const NONE: i32 = 0;
    pub const FAIL: i32 = 1;
    pub const OK: i32 = 2;
}
