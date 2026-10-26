//! Control channel (channel 0) message handler.
//!
//! Handles all control-plane messages: ping keepalive, service discovery, channel open/close,
//! audio/navigation focus, binding, and shutdown.
//!
//! Unlike the TS original (which threads a runtime `protobuf.Type` registry through every call
//! because protobufjs types are loaded dynamically), `aa-proto`'s prost-generated types are
//! concrete Rust structs with `Message::decode`/`.encode_to_vec()` directly available, so no
//! registry/state is needed here at all — `ControlChannel` carries no fields.
//!
//! Also unlike the TS original (an `EventEmitter`), messages that the session needs to react to
//! are returned as a `ControlEvent` from `handle_message` rather than fired as async events —
//! there is exactly one consumer (`Session`), so a direct return value is simpler than an event
//! bus. Immediate protocol replies (ping, audio focus, navigation focus, binding) are still sent
//! straight from within `handle_message` via the `send` callback, matching the original.

use prost::Message;

use aa_proto::aap_protobuf::service::control::message::{
    ChannelOpenRequest, ChannelOpenResponse, PingRequest, PingResponse, ServiceDiscoveryRequest,
};
use aa_proto::aap_protobuf::shared::MessageStatus;
use aa_proto::oaa::proto::enums::status::Enum as OaaStatus;
use aa_proto::oaa::proto::messages::{BindingRequest, BindingResponse};

use super::super::constants::{av_msg, ch, ctrl_msg, frame_flags};

/// Outbound wire write: (channel_id, flags, msg_id, data).
pub type SendFn<'a> = dyn FnMut(u8, u8, u16, &[u8]) + 'a;

#[derive(Debug)]
pub enum ControlEvent {
    ServiceDiscoveryRequest(ServiceDiscoveryRequest),
    ChannelOpenRequest { channel_id: i32 },
    Pong,
    VoiceSession(bool),
    Shutdown { reason: i32 },
    ShutdownComplete,
    None,
}

#[derive(Default)]
pub struct ControlChannel;

impl ControlChannel {
    pub fn handle_message(
        &mut self,
        msg_id: u16,
        payload: &[u8],
        send: &mut SendFn,
    ) -> ControlEvent {
        match msg_id {
            ctrl_msg::SERVICE_DISCOVERY_REQUEST => {
                let req = ServiceDiscoveryRequest::decode(payload).unwrap_or_default();
                println!(
                    "[ControlChannel] ServiceDiscoveryRequest device={:?} label={:?}",
                    req.device_name, req.label_text
                );
                ControlEvent::ServiceDiscoveryRequest(req)
            }

            ctrl_msg::CHANNEL_OPEN_RESPONSE => {
                if let Ok(resp) = ChannelOpenResponse::decode(payload) {
                    if resp.status != MessageStatus::StatusSuccess as i32 {
                        eprintln!(
                            "[ControlChannel] ChannelOpenResponse status={}",
                            resp.status
                        );
                    }
                }
                ControlEvent::None
            }

            ctrl_msg::CHANNEL_OPEN_REQUEST => match ChannelOpenRequest::decode(payload) {
                Ok(req) => ControlEvent::ChannelOpenRequest {
                    channel_id: req.service_id,
                },
                Err(_) => ControlEvent::None,
            },

            ctrl_msg::PING_REQUEST => {
                self.on_ping_request(payload, send);
                ControlEvent::None
            }

            ctrl_msg::PING_RESPONSE => ControlEvent::Pong,

            ctrl_msg::AUDIO_FOCUS_REQUEST => {
                self.on_audio_focus_request(payload, send);
                ControlEvent::None
            }

            ctrl_msg::NAVIGATION_FOCUS_REQUEST => {
                // Auto-grant navigation focus — echo the request payload back as the response
                // (NavigationFocusResponse has the same field structure as the request).
                send(
                    ch::CONTROL,
                    frame_flags::ENC_SIGNAL,
                    ctrl_msg::NAVIGATION_FOCUS_RESPONSE,
                    payload,
                );
                ControlEvent::None
            }

            ctrl_msg::SHUTDOWN_REQUEST => {
                let reason = if payload.len() >= 4 {
                    u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as i32
                } else {
                    0
                };
                ControlEvent::Shutdown { reason }
            }

            ctrl_msg::SHUTDOWN_RESPONSE => ControlEvent::ShutdownComplete,

            ctrl_msg::BINDING_REQUEST => {
                self.on_binding_request(payload, send);
                ControlEvent::None
            }

            ctrl_msg::VOICE_SESSION_NOTIFICATION => {
                let status = read_field1_varint_byte(payload);
                ControlEvent::VoiceSession(status == Some(1))
            }

            // AV setup requests arrive on control channel for some channel IDs; this shouldn't
            // arrive on channel 0, but handle defensively (matches TS behaviour: ignored).
            av_msg::SETUP_REQUEST => ControlEvent::None,

            _ => ControlEvent::None,
        }
    }

    /// CHANNEL_OPEN_RESPONSE always goes on ch=0 (control channel).
    pub fn send_channel_open_response(status: i32, send: &mut SendFn) {
        let buf = ChannelOpenResponse { status }.encode_to_vec();
        send(
            ch::CONTROL,
            frame_flags::ENC_CONTROL,
            ctrl_msg::CHANNEL_OPEN_RESPONSE,
            &buf,
        );
    }

    fn on_ping_request(&self, payload: &[u8], send: &mut SendFn) {
        let Ok(req) = PingRequest::decode(payload) else {
            eprintln!("[ControlChannel] ping parse error");
            return;
        };
        let buf = PingResponse {
            timestamp: req.timestamp,
            data: None,
        }
        .encode_to_vec();
        // PING_RESPONSE is plaintext per aasdk EncryptionType for PING_RESPONSE (ch=0, msgId=0x000c).
        send(
            ch::CONTROL,
            frame_flags::PLAINTEXT,
            ctrl_msg::PING_RESPONSE,
            &buf,
        );
    }

    fn on_audio_focus_request(&self, payload: &[u8], send: &mut SendFn) {
        // AudioFocusRequest:      field 1 = audio_focus_type (varint, AudioFocusRequestType)
        // AudioFocusNotification: field 1 = focus_state      (varint, AudioFocusStateType)
        //
        // Manually decode/encode, matching the wire shape directly rather than via a message
        // type (mirrors the TS original, which does the same for lack of an AudioFocus proto).
        //
        //   REQUEST type                → RESPONSE state
        //   GAIN (1)                    → STATE_GAIN (1)
        //   GAIN_TRANSIENT (2)          → STATE_GAIN_TRANSIENT (2)
        //   GAIN_TRANSIENT_MAY_DUCK (3) → STATE_GAIN_TRANSIENT (2)
        //   RELEASE (4)                 → STATE_LOSS (3)
        //   unknown / 0                 → STATE_LOSS (3)
        let focus_type = read_field1_varint_byte(payload).unwrap_or(0);
        let focus_state: u8 = match focus_type {
            1 => 1,
            2 | 3 => 2,
            _ => 3,
        };
        send(
            ch::CONTROL,
            frame_flags::ENC_SIGNAL,
            ctrl_msg::AUDIO_FOCUS_RESPONSE,
            &[0x08, focus_state],
        );
    }

    fn on_binding_request(&self, payload: &[u8], send: &mut SendFn) {
        let Ok(req) = BindingRequest::decode(payload) else {
            eprintln!("[ControlChannel] binding request error");
            return;
        };
        println!(
            "[ControlChannel] BindingRequest scan_codes={:?}",
            req.scan_codes
        );
        let buf = BindingResponse {
            status: Some(OaaStatus::Ok as i32),
            already_paired: None,
        }
        .encode_to_vec();
        send(
            ch::CONTROL,
            frame_flags::ENC_SIGNAL,
            ctrl_msg::BINDING_RESPONSE,
            &buf,
        );
    }
}

/// Reads a single-byte varint value out of a minimal `[tag=0x08][value]` field-1 encoding, as
/// used by several one-field control messages here that aren't worth round-tripping through a
/// full proto message type.
fn read_field1_varint_byte(payload: &[u8]) -> Option<u8> {
    if payload.len() >= 2 && payload[0] == 0x08 {
        Some(payload[1])
    } else {
        None
    }
}
