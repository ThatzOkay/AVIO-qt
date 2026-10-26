//! Audio channel handler (GAL types: MEDIA_AUDIO=4, SPEECH_AUDIO=5, SYSTEM_AUDIO=6).
//!
//! Receives PCM or AAC-LC frames from the phone and returns them as `AudioEvent::Pcm`. Sends
//! AVMediaAck for flow control (same as `VideoChannel`).
//!
//! Wire protocol (same as VideoChannel):
//!   Phone -> HU: AV_MEDIA_INDICATION (0x0001) — audio data
//!   Phone -> HU: AV_MEDIA_WITH_TIMESTAMP (0x0000) — audio data (legacy)
//!   Phone -> HU: SETUP_REQUEST (0x8000) — codec negotiation (handled generically in Session)
//!   HU -> Phone: SETUP_RESPONSE (0x8003) — accept setup (sent generically in Session)
//!   HU -> Phone: START_INDICATION (0x8001) — begin streaming
//!   HU -> Phone: AV_MEDIA_ACK (0x8004) — flow control

use super::super::constants::{av_msg, frame_flags};
use super::proto_enc::{decode_start, field_varint};

pub type SendFn<'a> = dyn FnMut(u8, u8, u16, &[u8]) + 'a;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioChannelType {
    Media,
    Speech,
    System,
}

#[derive(Debug)]
pub enum AudioEvent {
    Pcm { data: Vec<u8>, timestamp_ns: u64 },
    Start,
    Stop,
    None,
}

pub struct AudioChannel {
    channel_id: u8,
    channel_type: AudioChannelType,
    session: i32,
    sample_rate: u32,
    channel_count: u32,
}

impl AudioChannel {
    pub fn new(channel_id: u8, channel_type: AudioChannelType) -> Self {
        Self {
            channel_id,
            channel_type,
            session: 0,
            sample_rate: 48000,
            channel_count: 2,
        }
    }

    pub fn channel_type(&self) -> AudioChannelType {
        self.channel_type
    }

    pub fn handle_message(&mut self, msg_id: u16, payload: &[u8], send: &mut SendFn) -> AudioEvent {
        match msg_id {
            av_msg::AV_MEDIA_INDICATION => self.on_media_indication(payload, false, send),
            av_msg::AV_MEDIA_WITH_TIMESTAMP => self.on_media_indication(payload, true, send),

            av_msg::START_INDICATION => {
                if let Some(start) = decode_start(payload) {
                    self.session = start.session_id;
                }
                println!(
                    "[AudioChannel ch={}] stream started, session={}",
                    self.channel_id, self.session
                );
                AudioEvent::Start
            }

            av_msg::STOP_INDICATION => {
                println!("[AudioChannel ch={}] stream stopped", self.channel_id);
                AudioEvent::Stop
            }

            _ => AudioEvent::None,
        }
    }

    /// Called by Session when the phone's AVChannelSetupRequest arrives for this channel.
    pub fn handle_setup_request(&mut self, sample_rate: u32, channel_count: u32) {
        if sample_rate > 0 {
            self.sample_rate = sample_rate;
        }
        if channel_count > 0 {
            self.channel_count = channel_count;
        }
        println!(
            "[AudioChannel ch={}] setup {}Hz {}ch",
            self.channel_id, self.sample_rate, self.channel_count
        );
    }

    fn on_media_indication(
        &mut self,
        payload: &[u8],
        has_timestamp: bool,
        send: &mut SendFn,
    ) -> AudioEvent {
        let (timestamp_ns, data) = if has_timestamp && payload.len() >= 8 {
            let ts = u64::from_be_bytes(payload[0..8].try_into().expect("checked len >= 8"));
            (ts, payload[8..].to_vec())
        } else {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            (ts, payload.to_vec())
        };

        self.send_ack(send);
        AudioEvent::Pcm { data, timestamp_ns }
    }

    fn send_ack(&self, send: &mut SendFn) {
        let mut msg = field_varint(1, self.session as i64);
        msg.extend(field_varint(2, 1));
        send(
            self.channel_id,
            frame_flags::ENC_SIGNAL,
            av_msg::AV_MEDIA_ACK,
            &msg,
        );
    }
}
