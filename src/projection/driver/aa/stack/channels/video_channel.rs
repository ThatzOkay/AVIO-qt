//! Video channel handler — main display (`CH.VIDEO`=3) or cluster (`CH.CLUSTER_VIDEO`=19).
//!
//! Receives H.264/H.265 NAL units from the phone and returns them as `VideoEvent::Frame`.
//! Sends `AVMediaAck` for flow control on every received frame.
//!
//! Wire protocol:
//!   Phone -> HU: AV_MEDIA_INDICATION (0x0001) — H.264 data with timestamp
//!   Phone -> HU: AV_MEDIA_WITH_TIMESTAMP (0x0000) — H.264 data (legacy)
//!   Phone -> HU: SETUP_REQUEST (0x8000) — codec negotiation (handled generically in Session)
//!   HU -> Phone: SETUP_RESPONSE (0x8003) — accept setup (sent generically in Session)
//!   HU -> Phone: START_INDICATION (0x8001) — begin streaming
//!   HU -> Phone: AV_MEDIA_ACK (0x8004) — flow control
//!
//! Field notes:
//!   - H.264 data already has AnnexB start codes (00 00 00 01). Do NOT add more.
//!   - SPS/PPS arrives as AV_MEDIA_INDICATION with no timestamp — forward to the decoder first.
//!   - ACK every frame to avoid the phone triggering CAR_NOT_RESPONDING (>400 unacked).

use super::super::constants::{av_msg, frame_flags};
use super::proto_enc::{decode_start, field_varint, read_varint};

pub type SendFn<'a> = dyn FnMut(u8, u8, u16, &[u8]) + 'a;

#[derive(Debug)]
pub enum VideoEvent {
    /// H.264/H.265 NAL unit, ready for the decoder.
    Frame {
        data: Vec<u8>,
        timestamp_ns: u64,
    },
    /// Phone wants NATIVE/NATIVE_TRANSIENT focus — user wants the host UI.
    HostUiRequested,
    /// Phone confirmed/granted PROJECTED focus.
    VideoFocusProjected,
    None,
}

pub struct VideoChannel {
    channel_id: u8,
    session: i32,
    frame_count: u64,
}

impl VideoChannel {
    pub fn new(channel_id: u8) -> Self {
        Self {
            channel_id,
            session: 0,
            frame_count: 0,
        }
    }

    pub fn channel_id(&self) -> u8 {
        self.channel_id
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    pub fn handle_message(&mut self, msg_id: u16, payload: &[u8], send: &mut SendFn) -> VideoEvent {
        match msg_id {
            av_msg::AV_MEDIA_INDICATION => self.on_media_indication(payload, false, send),
            av_msg::AV_MEDIA_WITH_TIMESTAMP => self.on_media_indication(payload, true, send),

            av_msg::START_INDICATION => {
                if let Some(start) = decode_start(payload) {
                    self.session = start.session_id;
                }
                println!(
                    "[VideoChannel ch={}] stream started, session={}",
                    self.channel_id, self.session
                );
                VideoEvent::None
            }

            av_msg::STOP_INDICATION => {
                println!("[VideoChannel ch={}] stream stopped", self.channel_id);
                // The phone can stop the stream without ever sending a VIDEO_FOCUS_REQUEST
                // (e.g. ending the session, not just yielding focus transiently) - treat it the
                // same as HostUiRequested so the host UI actually comes back instead of staying
                // stuck showing a frozen last frame behind a transparent, now-stream-less window.
                VideoEvent::HostUiRequested
            }

            // Phone granted/revoked video focus — nothing to do for passthrough.
            av_msg::VIDEO_FOCUS_INDICATION => VideoEvent::None,

            av_msg::VIDEO_FOCUS_REQUEST => self.on_video_focus_request(payload, send),

            _ => VideoEvent::None,
        }
    }

    fn on_media_indication(
        &mut self,
        payload: &[u8],
        has_timestamp: bool,
        send: &mut SendFn,
    ) -> VideoEvent {
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

        self.frame_count += 1;
        self.send_ack(send);
        VideoEvent::Frame { data, timestamp_ns }
    }

    fn send_ack(&self, send: &mut SendFn) {
        // aap_protobuf.service.media.source.message.Ack:
        //   required int32  session_id           = 1;
        //   optional uint32 ack                  = 2;
        //   repeated uint64 receive_timestamp_ns = 3;
        let mut msg = field_varint(1, self.session as i64);
        msg.extend(field_varint(2, 1));
        send(
            self.channel_id,
            frame_flags::ENC_SIGNAL,
            av_msg::AV_MEDIA_ACK,
            &msg,
        );
    }

    fn on_video_focus_request(&self, payload: &[u8], send: &mut SendFn) -> VideoEvent {
        // VideoFocusRequestNotification {
        //   optional int32 disp_channel_id = 1 [deprecated];
        //   optional VideoFocusMode mode = 2;     // PROJECTED=1, NATIVE=2, NATIVE_TRANSIENT=3
        //   optional VideoFocusReason reason = 3; // UNKNOWN=0, PHONE_SCREEN_OFF=1, LAUNCH_NATIVE=2
        // }
        let mut mode: u32 = 1; // default PROJECTED if missing
        let mut off = 0usize;
        while off < payload.len() {
            let t = payload[off];
            off += 1;
            if t == 0x10 {
                let (v, n) = read_varint(payload, off);
                mode = v;
                off += n;
            } else {
                let (_, n) = read_varint(payload, off);
                off += n;
            }
        }

        // Echo back the mode actually granted, not a hardcoded PROJECTED — telling the phone it
        // kept PROJECTED focus when it just asked to relinquish it (e.g. its in-app close button
        // requesting NATIVE) desyncs its focus state from ours, so a later resume request goes
        // ignored since the phone never thought it lost focus in the first place.
        println!(
            "[VideoChannel ch={}] VideoFocusRequest mode={mode} -> granting",
            self.channel_id
        );
        send(
            self.channel_id,
            frame_flags::ENC_SIGNAL,
            av_msg::VIDEO_FOCUS_INDICATION,
            &[0x08, mode as u8],
        );

        if mode == 2 || mode == 3 {
            VideoEvent::HostUiRequested
        } else {
            VideoEvent::VideoFocusProjected
        }
    }
}
