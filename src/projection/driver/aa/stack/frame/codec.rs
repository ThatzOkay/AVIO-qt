//! AA wire-protocol frame codec.
//!
//! aasdk frame layout (channel + flags header is always plaintext, payload may be encrypted).
//! The size header has TWO forms depending on the frame type:
//!
//!   SHORT (BULK, MIDDLE, LAST):
//!     Byte 0:    channelId (u8)
//!     Byte 1:    flags     (u8)
//!     Bytes 2-3: payloadSize (u16 BE)
//!     Bytes 4..: payload (payloadSize bytes)
//!     -> total wire size = 4 + payloadSize
//!
//!   EXTENDED (FIRST only -- first fragment of a multi-frame message):
//!     Byte 0:    channelId
//!     Byte 1:    flags          (bit0=FIRST=1, bit1=LAST=0)
//!     Bytes 2-3: payloadSize    (u16 BE)  <- just THIS fragment's payload
//!     Bytes 4-7: totalSize      (u32 BE)  <- full reassembled message size
//!     Bytes 8..: payload (payloadSize bytes)
//!     -> total wire size = 8 + payloadSize
//!
//! Payload format (after size headers are consumed):
//!   For pre-TLS / plaintext frames the payload is:
//!     Bytes 0-1: messageId (u16 BE)
//!     Bytes 2..: protobuf data (or raw bytes for VERSION/SSL_HANDSHAKE)
//!   For encrypted post-TLS frames the payload IS one or more TLS records. Multi-frame TLS
//!   payloads must be reassembled into a single byte stream before being handed to the TLS layer.

use std::collections::HashMap;

pub const FRAME_HEADER_SHORT: usize = 4; // ch + flags + payloadSize(2)
pub const FRAME_HEADER_EXTENDED: usize = 8; // ch + flags + payloadSize(2) + totalSize(4)

#[derive(Debug, Clone)]
pub struct RawFrame {
    pub channel_id: u8,
    pub flags: u8,
    pub msg_id: u16,          // first 2 bytes of payload (after framing)
    pub payload: Vec<u8>,     // bytes AFTER the 2-byte msg_id (the actual proto data)
    pub raw_payload: Vec<u8>, // full payload including msg_id bytes
}

/// Encode a complete BULK frame ready to write to the TCP socket.
pub fn encode_frame(channel_id: u8, flags: u8, msg_id: u16, data: &[u8]) -> Vec<u8> {
    let mut full_payload = Vec::with_capacity(2 + data.len());
    full_payload.extend_from_slice(&msg_id.to_be_bytes());
    full_payload.extend_from_slice(data);

    let mut frame = Vec::with_capacity(FRAME_HEADER_SHORT + full_payload.len());
    frame.push(channel_id);
    frame.push(flags);
    frame.extend_from_slice(&(full_payload.len() as u16).to_be_bytes());
    frame.extend_from_slice(&full_payload);
    frame
}

struct FragmentState {
    parts: Vec<Vec<u8>>,
    total_size: u32,
}

/// Streaming frame parser.
///
/// Feed raw TCP bytes via `push()`, which returns any frames that completed as a result.
/// Handles fragmented TCP reads and multi-frame reassembly.
///
/// Multi-frame messages (FIRST -> MIDDLE* -> LAST) are reassembled into a single payload
/// before being returned. The FIRST fragment's EXTENDED size header announces `total_size`
/// (the full reassembled message size).
#[derive(Default)]
pub struct FrameParser {
    buf: Vec<u8>,
    // Per-channel reassembly state. The announced total_size comes from the EXTENDED size
    // header on the FIRST fragment.
    fragments: HashMap<u8, FragmentState>,
}

impl FrameParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<RawFrame> {
        self.buf.extend_from_slice(chunk);
        self.drain()
    }

    fn drain(&mut self) -> Vec<RawFrame> {
        let mut out = Vec::new();
        loop {
            if self.buf.len() < FRAME_HEADER_SHORT {
                break;
            }
            let channel_id = self.buf[0];
            let flags = self.buf[1];
            let is_first = flags & 0x01 != 0;
            let is_last = flags & 0x02 != 0;
            let is_extended = is_first && !is_last;
            let header_len = if is_extended {
                FRAME_HEADER_EXTENDED
            } else {
                FRAME_HEADER_SHORT
            };

            if self.buf.len() < header_len {
                break;
            }

            let payload_size = u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize;
            let total_frame = header_len + payload_size;
            if self.buf.len() < total_frame {
                break;
            }

            let announced_total_size = if is_extended {
                u32::from_be_bytes([self.buf[4], self.buf[5], self.buf[6], self.buf[7]])
            } else {
                0
            };

            let raw_payload = self.buf[header_len..total_frame].to_vec();
            self.buf.drain(0..total_frame);

            self.handle_frame(
                channel_id,
                flags,
                raw_payload,
                announced_total_size,
                &mut out,
            );
        }
        out
    }

    fn handle_frame(
        &mut self,
        channel_id: u8,
        flags: u8,
        payload: Vec<u8>,
        announced_total_size: u32,
        out: &mut Vec<RawFrame>,
    ) {
        let is_first = flags & 0x01 != 0;
        let is_last = flags & 0x02 != 0;

        // BULK -- single-frame message, emit immediately.
        if is_first && is_last {
            Self::emit(channel_id, flags, payload, out);
            return;
        }

        // FIRST -- start reassembly; total_size was already extracted from the EXTENDED
        // size header by drain().
        if is_first && !is_last {
            self.fragments.insert(
                channel_id,
                FragmentState {
                    parts: vec![payload],
                    total_size: announced_total_size,
                },
            );
            return;
        }

        // MIDDLE / LAST -- append.
        let Some(state) = self.fragments.get_mut(&channel_id) else {
            // Continuation with no first fragment on record — drop it.
            return;
        };
        state.parts.push(payload);

        if is_last {
            let state = self.fragments.remove(&channel_id).expect("checked above");
            let full: Vec<u8> = state.parts.into_iter().flatten().collect();
            debug_assert!(
                full.len() as u32 == state.total_size || state.total_size == 0,
                "ch={channel_id} reassembly size mismatch: got {}B, expected {}B",
                full.len(),
                state.total_size
            );
            Self::emit(channel_id, flags, full, out);
        }
    }

    fn emit(channel_id: u8, flags: u8, raw_payload: Vec<u8>, out: &mut Vec<RawFrame>) {
        if raw_payload.len() < 2 {
            return;
        }
        let msg_id = u16::from_be_bytes([raw_payload[0], raw_payload[1]]);
        let payload = raw_payload[2..].to_vec();
        out.push(RawFrame {
            channel_id,
            flags,
            msg_id,
            payload,
            raw_payload,
        });
    }
}
