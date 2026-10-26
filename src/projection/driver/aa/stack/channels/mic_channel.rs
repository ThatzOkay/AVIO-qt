//! Microphone channel handler — outbound counterpart of `AudioChannel`.
//!
//! Wire protocol on `CH.MIC_INPUT` (=9):
//!   Phone -> HU: SETUP_REQUEST (0x8000)
//!   HU -> Phone: SETUP_RESPONSE (0x8003) — accept setup (sent generically in Session)
//!   Phone -> HU: AV_INPUT_OPEN_REQUEST (0x8005) — MicrophoneRequest{open:true/false}
//!   HU -> Phone: AV_INPUT_OPEN_RESPONSE (0x8006) — MicrophoneResponse{status, session_id}
//!   HU -> Phone: START_INDICATION (0x8001) — HU is the sender on input channels
//!   HU -> Phone: AV_MEDIA_INDICATION (0x0001) — mic PCM frames (with timestamp)
//!   Phone -> HU: AV_MEDIA_ACK (0x8004) — flow control
//!   Phone -> HU: STOP_INDICATION (0x8002) — phone tears mic down

use std::collections::VecDeque;

use super::super::constants::{av_msg, frame_flags};
use super::proto_enc::{decode_fields, decode_varint_value, field_varint};

pub type SendFn<'a> = dyn FnMut(u8, u8, u16, &[u8]) + 'a;

const MAX_PENDING: usize = 64;

#[derive(Debug)]
pub enum MicEvent {
    /// Phone asked the HU to begin sending mic PCM.
    Start,
    /// Phone asked the HU to stop, or the phone closed the channel.
    Stop,
    None,
}

pub struct MicChannel {
    channel_id: u8,
    sample_rate: u32,
    channel_count: u32,
    session: i32, // HU-chosen session id, echoed by the phone in ACKs
    open: bool,   // true between OPEN(open=true) and OPEN(open=false) / STOP
    max_unacked: u32,
    unacked: u32,
    pending: VecDeque<(u64, Vec<u8>)>, // (timestamp_ns, data) backlog while unacked >= max
}

impl MicChannel {
    pub fn new(channel_id: u8) -> Self {
        Self {
            channel_id,
            sample_rate: 16000,
            channel_count: 1,
            session: 1,
            open: false,
            max_unacked: 1,
            unacked: 0,
            pending: VecDeque::new(),
        }
    }

    pub fn handle_message(&mut self, msg_id: u16, payload: &[u8], send: &mut SendFn) -> MicEvent {
        match msg_id {
            av_msg::SETUP_REQUEST => MicEvent::None, // handled generically in Session

            av_msg::AV_INPUT_OPEN_REQUEST => self.on_open_request(payload, send),

            av_msg::AV_MEDIA_ACK => {
                // Flow control — phone confirms it received a frame.
                if self.unacked > 0 {
                    self.unacked -= 1;
                }
                self.drain_pending(send);
                MicEvent::None
            }

            av_msg::STOP_INDICATION => {
                if self.open {
                    self.open = false;
                    self.unacked = 0;
                    self.pending.clear();
                    println!("[MicChannel] STOP_INDICATION — closing mic");
                    MicEvent::Stop
                } else {
                    MicEvent::None
                }
            }

            _ => MicEvent::None,
        }
    }

    /// Called by Session when the phone's AVChannelSetupRequest arrives.
    pub fn handle_setup_request(&mut self, sample_rate: u32, channel_count: u32) {
        if sample_rate > 0 {
            self.sample_rate = sample_rate;
        }
        if channel_count > 0 {
            self.channel_count = channel_count;
        }
        println!(
            "[MicChannel] setup {}Hz {}ch",
            self.sample_rate, self.channel_count
        );
    }

    /// Push a PCM chunk to the phone. Wraps in AV_MEDIA_WITH_TIMESTAMP with the timestamp
    /// prefix the phone-side decoder expects. No-op while the mic isn't open.
    pub fn push_pcm(&mut self, data: Vec<u8>, timestamp_ns: u64, send: &mut SendFn) {
        if !self.open {
            return;
        }
        if self.unacked >= self.max_unacked {
            self.pending.push_back((timestamp_ns, data));
            if self.pending.len() > MAX_PENDING {
                self.pending.pop_front();
            }
            return;
        }
        self.send_frame(&data, timestamp_ns, send);
    }

    fn drain_pending(&mut self, send: &mut SendFn) {
        while !self.pending.is_empty() && self.unacked < self.max_unacked {
            let (ts, data) = self.pending.pop_front().expect("checked non-empty above");
            self.send_frame(&data, ts, send);
        }
    }

    fn send_frame(&mut self, data: &[u8], timestamp_ns: u64, send: &mut SendFn) {
        // AV_MEDIA_WITH_TIMESTAMP layout: 8-byte BE timestamp + raw PCM samples.
        let mut out = Vec::with_capacity(8 + data.len());
        out.extend_from_slice(&timestamp_ns.to_be_bytes());
        out.extend_from_slice(data);
        send(
            self.channel_id,
            frame_flags::ENC_SIGNAL,
            av_msg::AV_MEDIA_WITH_TIMESTAMP,
            &out,
        );
        self.unacked += 1;
    }

    fn on_open_request(&mut self, payload: &[u8], send: &mut SendFn) -> MicEvent {
        // MicrophoneRequest: f1 bool open, f2 anc, f3 ec, f4 max_unacked
        let mut open = false;
        for f in decode_fields(payload) {
            if f.field == 1 && f.wire == 0 {
                open = decode_varint_value(&f.bytes) != 0;
            } else if f.field == 4 && f.wire == 0 {
                self.max_unacked = decode_varint_value(&f.bytes).max(1);
            }
        }
        println!(
            "[MicChannel] OPEN_REQUEST open={open} max_unacked={}",
            self.max_unacked
        );

        // MicrophoneResponse: f1 status (0 = OK), f2 session_id
        let mut resp = field_varint(1, 0);
        resp.extend(field_varint(2, self.session as i64));
        send(
            self.channel_id,
            frame_flags::ENC_SIGNAL,
            av_msg::AV_INPUT_OPEN_RESPONSE,
            &resp,
        );

        if open && !self.open {
            self.open = true;
            self.unacked = 0;
            self.pending.clear();

            // HU-sent START_INDICATION on input channels: { session_id, configuration_index=0 }
            let mut start = field_varint(1, self.session as i64);
            start.extend(field_varint(2, 0));
            send(
                self.channel_id,
                frame_flags::ENC_SIGNAL,
                av_msg::START_INDICATION,
                &start,
            );

            println!("[MicChannel] mic open, session={}", self.session);
            MicEvent::Start
        } else if !open && self.open {
            self.open = false;
            self.unacked = 0;
            self.pending.clear();
            println!("[MicChannel] mic close");
            MicEvent::Stop
        } else {
            MicEvent::None
        }
    }
}
