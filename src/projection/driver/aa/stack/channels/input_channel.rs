//! Input channel (`CH.INPUT` = 8). HU -> Phone direction only — touch, rotary, and hardware
//! button/key events. The one Phone -> HU message on this channel (KEY_BINDING_REQUEST) is
//! handled directly in `Session`, matching the reference (it's a fixed ack, not stateful).
//!
//! Wire protocol (aasdk InputMessageId):
//!   INPUT_MESSAGE_INPUT_REPORT = 32769 (0x8001) — touch / key / etc. events
//!
//! Proto schema (aap_protobuf.service.inputsource.message):
//!
//!   message InputReport {
//!     required uint64 timestamp        = 1;   // microseconds
//!     optional TouchEvent touch_event  = 3;
//!     optional KeyEvent   key_event    = 4;
//!     // (absolute_event=5, relative_event=6, touchpad_event=7 — unused here)
//!   }
//!
//!   message TouchEvent {
//!     repeated Pointer    pointer_data = 1;
//!     message Pointer { required uint32 x = 1; required uint32 y = 2; required uint32 pointer_id = 3; }
//!     optional uint32        action_index = 2;
//!     optional PointerAction action       = 3;
//!   }
//!
//!   enum PointerAction { ACTION_DOWN=0; ACTION_UP=1; ACTION_MOVED=2; ACTION_POINTER_DOWN=5; ACTION_POINTER_UP=6; }
//!
//!   message KeyEvent {
//!     repeated Key keys = 1;
//!     message Key { required uint32 keycode = 1; required bool down = 2; required uint32 metastate = 3; optional bool longpress = 4; }
//!   }
//!
//! Reference encoder: openauto InputSourceService.cpp::onTouchEvent / onButtonEvent.
//! Timestamps are MICROSECONDS as varint.

use super::proto_enc::{field_len_delim, field_varint};

pub mod input_msg {
    pub const INPUT_REPORT: u16 = 0x8001; // = 32769 (INPUT_MESSAGE_INPUT_REPORT)
    pub const KEY_BINDING_REQUEST: u16 = 0x8002; // = 32770, phone -> HU
    pub const KEY_BINDING_RESPONSE: u16 = 0x8003; // = 32771, HU -> phone
    #[allow(dead_code)] // not sent anywhere yet — no feedback loop built
    pub const INPUT_FEEDBACK: u16 = 0x8004; // = 32772
}

/// PointerAction enum values per aasdk PointerAction.proto.
pub mod touch_action {
    pub const DOWN: u32 = 0;
    pub const UP: u32 = 1;
    pub const MOVED: u32 = 2;
    pub const POINTER_DOWN: u32 = 5;
    pub const POINTER_UP: u32 = 6;
}

// Android KeyEvent.KEYCODE_* values + AA-specific extensions, mirroring
// `aap_protobuf/oaa/av/AndroidKeycodeEnum.proto`.
//
// IMPORTANT: a key only reaches the phone if its code is in the `keycodes_supported` list
// advertised in the SDR InputSourceService (see `service_discovery.rs`) — anything missing
// there is silently dropped phone-side.
#[allow(dead_code)] // most of these aren't wired to a UI/HW input source yet
pub mod button_key {
    // System
    pub const UNKNOWN: u32 = 0;
    pub const HOME: u32 = 3;
    pub const BACK: u32 = 4;
    // Phone call
    pub const PHONE_ACCEPT: u32 = 5;
    pub const PHONE_DECLINE: u32 = 6;
    // Numeric (DTMF / dialer pad)
    pub const KEY_0: u32 = 7;
    pub const KEY_1: u32 = 8;
    pub const KEY_2: u32 = 9;
    pub const KEY_3: u32 = 10;
    pub const KEY_4: u32 = 11;
    pub const KEY_5: u32 = 12;
    pub const KEY_6: u32 = 13;
    pub const KEY_7: u32 = 14;
    pub const KEY_8: u32 = 15;
    pub const KEY_9: u32 = 16;
    pub const KEY_STAR: u32 = 17;
    pub const KEY_POUND: u32 = 18;
    // D-PAD navigation
    pub const DPAD_UP: u32 = 19;
    pub const DPAD_DOWN: u32 = 20;
    pub const DPAD_LEFT: u32 = 21;
    pub const DPAD_RIGHT: u32 = 22;
    pub const DPAD_CENTER: u32 = 23;
    // Volume (informational — NOT advertised)
    pub const VOLUME_UP: u32 = 24;
    pub const VOLUME_DOWN: u32 = 25;
    pub const POWER: u32 = 26;
    // General
    pub const ENTER: u32 = 66;
    pub const HEADSETHOOK: u32 = 79;
    pub const MENU: u32 = 82;
    pub const SEARCH: u32 = 84;
    // Media transport
    pub const MEDIA_PLAY_PAUSE: u32 = 85;
    pub const MEDIA_STOP: u32 = 86;
    pub const MEDIA_NEXT: u32 = 87;
    pub const MEDIA_PREV: u32 = 88;
    pub const MEDIA_REWIND: u32 = 89;
    pub const MEDIA_FAST_FWD: u32 = 90;
    pub const MUTE: u32 = 91;
    pub const ESCAPE: u32 = 111;
    pub const MEDIA_PLAY: u32 = 126;
    pub const MEDIA_PAUSE: u32 = 127;
    pub const VOLUME_MUTE: u32 = 164;
    // Voice / assistant
    pub const ASSIST: u32 = 219;
    pub const VOICE_ASSIST: u32 = 231;
    // Map / list navigation
    pub const NAVIGATE_PREVIOUS: u32 = 260;
    pub const NAVIGATE_NEXT: u32 = 261;
    pub const NAVIGATE_IN: u32 = 262;
    pub const NAVIGATE_OUT: u32 = 263;
    // AA-specific extensions
    pub const ROTARY_CONTROLLER: u32 = 65536;
    pub const MEDIA: u32 = 65537;
    pub const TERTIARY_BUTTON: u32 = 65543;
    pub const TURN_CARD: u32 = 65544;
}

#[derive(Debug, Clone, Copy)]
pub struct TouchPointer {
    /// Absolute X in advertised touchscreen pixel space.
    pub x: u32,
    /// Absolute Y in advertised touchscreen pixel space.
    pub y: u32,
    /// Per-finger identifier (0 for single-touch; stable across DOWN->MOVED->UP).
    pub id: u32,
}

fn timestamp_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// Builds an `InputReport` touch event. `action_index` is the index in `pointers` of the
/// pointer that triggered `action` — only meaningful for POINTER_DOWN/POINTER_UP, ignored by AA
/// for plain DOWN/MOVED/UP. Returns `None` if `pointers` is empty (nothing to send).
pub fn build_touch_report(
    action: u32,
    pointers: &[TouchPointer],
    action_index: u32,
) -> Option<Vec<u8>> {
    if pointers.is_empty() {
        return None;
    }

    // TouchEvent.Pointer (field 1, repeated)
    let mut touch_event_buf = Vec::new();
    for p in pointers {
        let mut pointer_buf = field_varint(1, p.x as i64);
        pointer_buf.extend(field_varint(2, p.y as i64));
        pointer_buf.extend(field_varint(3, p.id as i64));
        touch_event_buf.extend(field_len_delim(1, &pointer_buf));
    }
    // TouchEvent.action_index (field 2) + action (field 3)
    touch_event_buf.extend(field_varint(2, action_index as i64));
    touch_event_buf.extend(field_varint(3, action as i64));

    // InputReport.timestamp (field 1, uint64 varint) + touch_event (field 3)
    let mut msg = field_varint(1, timestamp_micros() as i64);
    msg.extend(field_len_delim(3, &touch_event_buf));
    Some(msg)
}

/// Builds an `InputReport` rotary-controller delta event. AA's list/picker views consume
/// RelativeEvent.delta to scroll through items. `direction`: -1 = previous (left/up), +1 = next
/// (right/down).
pub fn build_rotary_report(direction: i32) -> Vec<u8> {
    // RelativeEvent.Rel: keycode (field 1), delta (field 2).
    let mut rel_buf = field_varint(1, button_key::ROTARY_CONTROLLER as i64);
    rel_buf.extend(field_varint(2, direction as i64));
    // RelativeEvent.data (field 1, repeated Rel)
    let relative_event_buf = field_len_delim(1, &rel_buf);
    // InputReport.timestamp (field 1) + relative_event (field 6)
    let mut msg = field_varint(1, timestamp_micros() as i64);
    msg.extend(field_len_delim(6, &relative_event_buf));
    msg
}

/// Builds an `InputReport` hardware key event covering one or more simultaneous keycodes (all
/// packed into the same `KeyEvent.keys` repeated field). `down`: true on press, false on
/// release — the phone expects a DOWN+UP pair. Returns `None` if `key_codes` is empty.
pub fn build_button_report(key_codes: &[u32], down: bool, longpress: bool) -> Option<Vec<u8>> {
    if key_codes.is_empty() {
        return None;
    }

    // KeyEvent.keys (field 1, repeated Key). All four fields written explicitly to match
    // openauto's InputSourceService::onButtonEvent — some AA versions require longpress to be
    // present even when false.
    let mut key_event_buf = Vec::new();
    for &kc in key_codes {
        let mut key_buf = field_varint(1, kc as i64); // keycode
        key_buf.extend(field_varint(2, if down { 1 } else { 0 })); // down (bool wire = varint)
        key_buf.extend(field_varint(3, 0)); // metastate — no modifiers
        key_buf.extend(field_varint(4, if longpress { 1 } else { 0 })); // longpress
        key_event_buf.extend(field_len_delim(1, &key_buf));
    }

    // InputReport.timestamp (field 1) + key_event (field 4)
    let mut msg = field_varint(1, timestamp_micros() as i64);
    msg.extend(field_len_delim(4, &key_event_buf));
    Some(msg)
}
