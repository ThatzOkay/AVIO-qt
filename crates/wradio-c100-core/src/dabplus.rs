//! DAB+ superframe sync, Reed-Solomon correction, and access-unit (AU)
//! splitting -- the MSC-stream-audio decode path for `ascty == 0x3F`
//! (DAB+/HE-AACv2) components, ported from
//! `dabplusservicecomponentdecoder.cpp/h`.
//!
//! Unlike the reference (a background `std::thread` fed by a
//! `ConcurrentQueue<vector<uint8_t>>`, per `DabPlusServiceComponentDecoder`'s
//! constructor/`processData`), this is a plain synchronous state machine:
//! feed raw MSC bytes into [`DabPlusDecoder::push_msc_data`] and get back
//! zero or more completed [`DabPlusOutput`]s (normally one per 5 synced
//! 24ms frames = 120ms of audio). The caller (a later stage's async device
//! task, per the port plan's Async/runtime section) owns the actual
//! task/channel plumbing; this module has none of its own.
//!
//! The classic (non-DAB+) MPEG Layer II audio path
//! (`dabmpegservicecomponentdecoder.cpp`, selected for `ascty == 0x00`) is
//! out of scope, matching `RUST_PORT_PROMPT.md`'s "DAB+ audio path" bullet
//! -- only DAB+ is ported.

use crate::rs::{self, RsCodec};

/// Registered Audio Service Component Type for DAB+ (HE-AAC v2), ETSI
/// TS 101 756 table 9 -- `dabservicecomponentmscstreamaudio.cpp` checks
/// `m_ascTy == 0x3F` to select this decoder in the first place, and the
/// reference echoes the same value back out as this decoder's own type on
/// every completed superframe (`m_audioDataDispatcher.invoke(auBuff, 63,
/// ...)`).
pub const ASCTY_DAB_PLUS: u8 = 0x3F;

/// One completed 120ms DAB+ superframe's worth of decoded output.
/// Mirrors the reference's `AUDIO_COMPONENT_DATA_CALLBACK` payload
/// (`const vector<uint8_t>&, int ascty, int channels, int sampleRate, bool
/// sbrUsed, bool psUsed`) plus the separately-dispatched PAD bytes
/// (`PAD_DATA_CALLBACK`, one call per AU that carries X-PAD) and basic RS
/// decode stats (not surfaced by the reference as data, only logged --
/// kept here since a caller wiring up signal-quality reporting, a later
/// phase, will want them without re-deriving them from raw AU CRC
/// failures).
#[derive(Debug, Clone)]
pub struct DabPlusOutput {
    /// Raw AAC access units for this superframe, concatenated back-to-back
    /// (each with its own 2-byte CRC and any leading X-PAD already
    /// stripped) -- an AU whose CRC failed is dropped entirely rather than
    /// included corrupted, matching the reference. AAC raw_data_blocks are
    /// self-delimiting in the bitstream syntax itself, so a decoder that
    /// parses AAC syntax directly (e.g. libfdk-aac's streaming API) can
    /// consume this buffer as-is without needing external frame
    /// boundaries -- matching how the reference hands its own equivalent
    /// concatenated `auBuff` to a single callback per superframe.
    ///
    /// For a consumer that instead needs each AU's own bytes/boundaries
    /// (e.g. building ADTS/LOAS framing for a generic demuxer that does
    /// *not* parse raw AAC syntax itself, such as `ffmpeg`/`ffplay`'s CLI),
    /// see [`DabPlusOutput::access_unit_lengths`].
    pub access_units: Vec<u8>,
    /// Byte length of each entry in [`DabPlusOutput::access_units`], in
    /// order (summing to `access_units.len()`) -- lets a caller recover
    /// per-AU slices without re-parsing AAC syntax. Not part of the
    /// reference (its callback never exposed per-AU boundaries either,
    /// see `access_units`'s own doc comment for why that's fine for a
    /// syntax-aware decoder) -- purely additive for callers that need them.
    pub access_unit_lengths: Vec<u16>,
    /// Raw X-PAD bytes for each AU in this superframe that carried any
    /// (AU start-of-frame type `0x04`), in AU order. Parsing this into
    /// Dynamic Label / MOT slideshow data is a later phase (`pad.rs`).
    pub pad_units: Vec<Vec<u8>>,
    pub ascty: u8,
    pub channels: u8,
    pub sample_rate: u32,
    pub sbr_used: bool,
    pub ps_used: bool,
    /// Total Reed-Solomon-corrected symbol count across every interleaved
    /// row of this superframe.
    pub rs_corrected_symbols: u32,
    /// Whether at least one interleaved row had more errors than the code
    /// could correct (audio for this superframe may be corrupted/dropped
    /// AUs as a result -- their CRC checks will generally also fail).
    pub rs_uncorrectable: bool,
}

// ---------------------------------------------------------------------
// Firecode frame sync
// ---------------------------------------------------------------------

/// `DabPlusServiceComponentDecoder::FIRECODE_TABLE`, transcribed verbatim.
const FIRECODE_TABLE: [u16; 256] = [
    0x0000, 0x782f, 0xf05e, 0x8871, 0x9893, 0xe0bc, 0x68cd, 0x10e2, 0x4909, 0x3126, 0xb957, 0xc178,
    0xd19a, 0xa9b5, 0x21c4, 0x59eb, 0x9212, 0xea3d, 0x624c, 0x1a63, 0x0a81, 0x72ae, 0xfadf, 0x82f0,
    0xdb1b, 0xa334, 0x2b45, 0x536a, 0x4388, 0x3ba7, 0xb3d6, 0xcbf9, 0x5c0b, 0x2424, 0xac55, 0xd47a,
    0xc498, 0xbcb7, 0x34c6, 0x4ce9, 0x1502, 0x6d2d, 0xe55c, 0x9d73, 0x8d91, 0xf5be, 0x7dcf, 0x05e0,
    0xce19, 0xb636, 0x3e47, 0x4668, 0x568a, 0x2ea5, 0xa6d4, 0xdefb, 0x8710, 0xff3f, 0x774e, 0x0f61,
    0x1f83, 0x67ac, 0xefdd, 0x97f2, 0xb816, 0xc039, 0x4848, 0x3067, 0x2085, 0x58aa, 0xd0db, 0xa8f4,
    0xf11f, 0x8930, 0x0141, 0x796e, 0x698c, 0x11a3, 0x99d2, 0xe1fd, 0x2a04, 0x522b, 0xda5a, 0xa275,
    0xb297, 0xcab8, 0x42c9, 0x3ae6, 0x630d, 0x1b22, 0x9353, 0xeb7c, 0xfb9e, 0x83b1, 0x0bc0, 0x73ef,
    0xe41d, 0x9c32, 0x1443, 0x6c6c, 0x7c8e, 0x04a1, 0x8cd0, 0xf4ff, 0xad14, 0xd53b, 0x5d4a, 0x2565,
    0x3587, 0x4da8, 0xc5d9, 0xbdf6, 0x760f, 0x0e20, 0x8651, 0xfe7e, 0xee9c, 0x96b3, 0x1ec2, 0x66ed,
    0x3f06, 0x4729, 0xcf58, 0xb777, 0xa795, 0xdfba, 0x57cb, 0x2fe4, 0x0803, 0x702c, 0xf85d, 0x8072,
    0x9090, 0xe8bf, 0x60ce, 0x18e1, 0x410a, 0x3925, 0xb154, 0xc97b, 0xd999, 0xa1b6, 0x29c7, 0x51e8,
    0x9a11, 0xe23e, 0x6a4f, 0x1260, 0x0282, 0x7aad, 0xf2dc, 0x8af3, 0xd318, 0xab37, 0x2346, 0x5b69,
    0x4b8b, 0x33a4, 0xbbd5, 0xc3fa, 0x5408, 0x2c27, 0xa456, 0xdc79, 0xcc9b, 0xb4b4, 0x3cc5, 0x44ea,
    0x1d01, 0x652e, 0xed5f, 0x9570, 0x8592, 0xfdbd, 0x75cc, 0x0de3, 0xc61a, 0xbe35, 0x3644, 0x4e6b,
    0x5e89, 0x26a6, 0xaed7, 0xd6f8, 0x8f13, 0xf73c, 0x7f4d, 0x0762, 0x1780, 0x6faf, 0xe7de, 0x9ff1,
    0xb015, 0xc83a, 0x404b, 0x3864, 0x2886, 0x50a9, 0xd8d8, 0xa0f7, 0xf91c, 0x8133, 0x0942, 0x716d,
    0x618f, 0x19a0, 0x91d1, 0xe9fe, 0x2207, 0x5a28, 0xd259, 0xaa76, 0xba94, 0xc2bb, 0x4aca, 0x32e5,
    0x6b0e, 0x1321, 0x9b50, 0xe37f, 0xf39d, 0x8bb2, 0x03c3, 0x7bec, 0xec1e, 0x9431, 0x1c40, 0x646f,
    0x748d, 0x0ca2, 0x84d3, 0xfcfc, 0xa517, 0xdd38, 0x5549, 0x2d66, 0x3d84, 0x45ab, 0xcdda, 0xb5f5,
    0x7e0c, 0x0623, 0x8e52, 0xf67d, 0xe69f, 0x9eb0, 0x16c1, 0x6eee, 0x3705, 0x4f2a, 0xc75b, 0xbf74,
    0xaf96, 0xd7b9, 0x5fc8, 0x27e7,
];

/// `DabPlusServiceComponentDecoder::CHECK_FIRECODE`. `frame` must be at
/// least 11 bytes.
fn check_firecode(frame: &[u8]) -> bool {
    let mut first_state: u16 = ((frame[2] as u16) << 8) | frame[3] as u16;

    for &byte in &frame[4..11] {
        let sec_state = FIRECODE_TABLE[(first_state >> 8) as usize];
        first_state =
            ((sec_state & 0x00ff) ^ byte as u16) | ((sec_state ^ (first_state << 8)) & 0xff00);
    }
    for &byte in &frame[0..2] {
        let sec_state = FIRECODE_TABLE[(first_state >> 8) as usize];
        first_state =
            ((sec_state & 0x00ff) ^ byte as u16) | ((sec_state ^ (first_state << 8)) & 0xff00);
    }

    first_state == 0
}

/// Byte-stream frame synchronizer: scans raw MSC bytes for `frame_size`-byte
/// windows whose Firecode checks out, mirroring
/// `DabPlusServiceComponentDecoder::synchronizeData`'s scan/resync state
/// machine (a provisional sync lasting 5 frames before it re-validates via
/// Firecode again, rather than trusting frame boundaries forever once
/// found).
struct FrameSync {
    frame_size: usize,
    buffer: Vec<u8>,
    sync: bool,
    frame_count: u8,
}

impl FrameSync {
    fn new(frame_size: usize) -> Self {
        Self {
            frame_size,
            buffer: Vec::new(),
            sync: false,
            frame_count: 0,
        }
    }

    /// Appends `input` to any buffered leftover bytes and returns every
    /// `frame_size`-byte frame found, in order.
    fn push(&mut self, input: &[u8]) -> Vec<Vec<u8>> {
        let mut data = std::mem::take(&mut self.buffer);
        data.extend_from_slice(input);

        let mut frames = Vec::new();
        let mut pos = 0usize;
        // Mirrors the reference's `unsyncIter + frameSize < data.end()`
        // bound exactly (strict `<`, not `<=`).
        while pos + self.frame_size < data.len() {
            if self.sync {
                while pos + self.frame_size < data.len() {
                    frames.push(data[pos..pos + self.frame_size].to_vec());
                    pos += self.frame_size;
                    self.frame_count += 1;
                    if self.frame_count == 5 {
                        self.sync = false;
                        self.frame_count = 0;
                        break;
                    }
                }
                continue;
            }

            let window = &data[pos..pos + self.frame_size];
            // A window starting with two zero bytes is excluded from the
            // Firecode check rather than merely expected to fail it: an
            // all-zero 11-byte prefix is (by construction of this
            // particular Firecode table) itself a passing "codeword",
            // which would otherwise false-sync on padding/silence.
            if !(window[0] == 0x00 && window[1] == 0x00) && check_firecode(window) {
                frames.push(window.to_vec());
                self.frame_count += 1;
                self.sync = true;
                pos += self.frame_size;
                continue;
            }

            pos += 1;
        }

        self.buffer = data[pos..].to_vec();
        frames
    }
}

// ---------------------------------------------------------------------
// Superframe assembly, RS correction, and AU splitting
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
struct SuperFrame {
    data: Vec<u8>,
    num_aus: u8,
    au_lengths: Vec<u16>,
    au_starts: Vec<u16>,
    sbr_used: bool,
    ps_used: bool,
    sampling_rate: u32,
    channels: u8,
}

/// DAB+ superframe sync + RS decode + AU splitting state machine.
/// Mirrors `DabPlusServiceComponentDecoder` (minus its background-thread
/// plumbing -- see this module's doc comment).
pub struct DabPlusDecoder {
    sync: FrameSync,
    rs: RsCodec,
    /// `(subChannelBitrate / 8) * 110` -- the useful (non-parity) length of
    /// one superframe, also used as an AU-start sanity bound.
    superframe_size: usize,
    current: SuperFrame,
    is_sync: bool,
    superframe_count: u32,
}

impl DabPlusDecoder {
    /// `subchannel_bitrate_kbits` is the component's subchannel bitrate in
    /// kbit/s (`DabServiceComponent::getSubchannelBitrate`, from FIG 0/1).
    /// Unlike the reference (whose `m_frameSize`/`m_superFrameSize` start
    /// at 0 and are only set once `setSubchannelBitrate` is called some
    /// time after construction, with `componentDataInput` a no-op until
    /// then), this takes the bitrate up front -- by the time this crate's
    /// MSC demux stage is ready to construct a decoder for a component, the
    /// bitrate is already known from the ensemble database, so there's no
    /// equivalent "not yet configured" state to model.
    pub fn new(subchannel_bitrate_kbits: u16) -> Self {
        let frame_size = subchannel_bitrate_kbits as usize * 3;
        let superframe_size = (subchannel_bitrate_kbits as usize / 8) * 110;
        Self {
            sync: FrameSync::new(frame_size),
            rs: RsCodec::new(),
            superframe_size,
            current: SuperFrame::default(),
            is_sync: false,
            superframe_count: 0,
        }
    }

    /// `DabPlusServiceComponentDecoder::componentDataInput` +
    /// `processData`'s per-frame loop body, combined: feeds raw MSC bytes
    /// through frame sync, then through superframe assembly, returning
    /// every superframe completed as a result (normally 0 or 1).
    pub fn push_msc_data(&mut self, data: &[u8]) -> Vec<DabPlusOutput> {
        let frames = self.sync.push(data);
        frames
            .into_iter()
            .filter_map(|frame| self.process_frame(frame))
            .collect()
    }

    fn process_frame(&mut self, frame: Vec<u8>) -> Option<DabPlusOutput> {
        if self.superframe_count == 0 {
            return self.start_superframe(&frame);
        }

        if self.superframe_count > 5 {
            // Superframe damaged: lost sync partway through 5 frames.
            self.is_sync = false;
            self.superframe_count = 0;
        }

        if !self.is_sync {
            return None;
        }

        self.current.data.extend_from_slice(&frame);
        self.superframe_count += 1;

        if self.superframe_count != 5 {
            return None;
        }

        self.superframe_count = 0;
        self.is_sync = false;
        self.finish_superframe()
    }

    /// Parses frame 0's superframe header (AAC params + AU start table)
    /// and, if valid, begins collecting the remaining 4 frames.
    fn start_superframe(&mut self, frame: &[u8]) -> Option<DabPlusOutput> {
        if !check_firecode(frame) {
            return None;
        }

        self.current = SuperFrame::default();

        // 2-byte Firecode preceding this header.
        let dac_rate = (frame[2] & 0x40) != 0; // dacRate ? 48000 Hz : 32000 Hz
        let sbr = (frame[2] & 0x20) != 0;
        let chan_mode = (frame[2] & 0x10) != 0; // chanMode ? stereo : mono
        let ps = (frame[2] & 0x08) != 0;
        // mpegSurCfg = frame[2] & 0x07 -- parsed by the reference too, but
        // never actually consumed there either.

        self.current.sbr_used = sbr;
        self.current.ps_used = ps;
        self.current.channels = if chan_mode { 2 } else { 1 };
        self.current.sampling_rate = if dac_rate { 48000 } else { 32000 };

        self.current.num_aus = match (dac_rate, sbr) {
            (false, true) => 2,  // 16 kHz AAC core + SBR, 60ms/AU
            (true, true) => 3,   // 24 kHz AAC core + SBR, 40ms/AU
            (false, false) => 4, // 32 kHz AAC core, 30ms/AU
            (true, false) => 6,  // 48 kHz AAC core, 20ms/AU
        };

        let first_au_start = match self.current.num_aus {
            2 => 5,
            3 => 6,
            4 => 8,
            6 => 11,
            _ => unreachable!("num_aus is only ever one of 2/3/4/6, set immediately above"),
        };
        self.current.au_starts.push(first_au_start);

        // Remaining AU start positions: a run of 12-bit big-endian values
        // packed two-per-three-bytes, starting at frame[3].
        let mut byte_pos = 3usize;
        for i in 1..self.current.num_aus {
            let au_start: u16 = if i % 2 == 1 {
                let hi = frame[byte_pos] as u16;
                byte_pos += 1;
                let lo = (frame[byte_pos] as u16 & 0xF0) >> 4;
                (hi << 4) | lo
            } else {
                let hi = frame[byte_pos] as u16 & 0x0F;
                byte_pos += 1;
                let lo = frame[byte_pos] as u16;
                byte_pos += 1;
                (hi << 8) | lo
            };

            let prev = self.current.au_starts[(i - 1) as usize];
            if au_start as usize > self.superframe_size || au_start <= prev {
                // Bad AU-start table: wait for the next frame 0 candidate
                // instead of using this one (matches the reference's
                // `continue` back to the outer frame loop, leaving
                // `m_dabSuperFrameCount`/`m_isSync` untouched at 0/false).
                return None;
            }
            self.current.au_starts.push(au_start);
        }

        self.current.au_starts.push(self.superframe_size as u16);
        for i in 0..self.current.num_aus as usize {
            self.current
                .au_lengths
                .push(self.current.au_starts[i + 1] - self.current.au_starts[i]);
        }

        self.current.data.extend_from_slice(frame);
        self.superframe_count = 1;
        self.is_sync = true;
        None
    }

    /// Reed-Solomon-decodes the fully-assembled superframe and splits it
    /// into access units, mirroring the tail of `processData` (the
    /// `m_dabSuperFrameCount == 5` branch).
    fn finish_superframe(&mut self) -> Option<DabPlusOutput> {
        let rs_stats = self.rs_decode_superframe();

        if self.current.data.len() < self.superframe_size {
            return None;
        }

        let mut access_units = Vec::new();
        let mut access_unit_lengths = Vec::new();
        let mut pad_units = Vec::new();

        for i in 0..self.current.num_aus as usize {
            let start = self.current.au_starts[i] as usize;
            let len = self.current.au_lengths[i] as usize;
            let au = &self.current.data[start..start + len];

            if !crate::tables::crc_ccitt_check(au) {
                // CRC failed: this AU is dropped entirely, matching the
                // reference (it only logs and continues).
                continue;
            }

            let mut au_start = start;
            let mut au_len = len;

            // AU start-of-frame type `0x04` (top 3 bits of the first byte)
            // marks X-PAD data prefixing this AU -- transcribed as the
            // reference has it; there's no in-source comment naming why
            // `4` specifically.
            if (self.current.data[au_start] >> 5) & 0x07 == 0x04 {
                let mut pad_data_start = 2usize;
                let mut pad_data_len = self.current.data[au_start + 1] as usize;
                if pad_data_len == 0xFF {
                    pad_data_len += self.current.data[au_start + 2] as usize;
                    pad_data_start += 1;
                }

                pad_units.push(
                    self.current.data
                        [au_start + pad_data_start..au_start + pad_data_start + pad_data_len]
                        .to_vec(),
                );

                au_start += pad_data_start + pad_data_len;
                au_len -= pad_data_start + pad_data_len;
            }

            // -2 to cut off the AU's own trailing CRC.
            access_units.extend_from_slice(&self.current.data[au_start..au_start + au_len - 2]);
            access_unit_lengths.push((au_len - 2) as u16);
        }

        Some(DabPlusOutput {
            access_units,
            access_unit_lengths,
            pad_units,
            ascty: ASCTY_DAB_PLUS,
            channels: self.current.channels,
            sample_rate: self.current.sampling_rate,
            sbr_used: self.current.sbr_used,
            ps_used: self.current.ps_used,
            rs_corrected_symbols: rs_stats.0,
            rs_uncorrectable: rs_stats.1,
        })
    }

    /// `RSDecoder::DecodeSuperframe`: de-interleaves the superframe into
    /// `rs::BLOCK_LEN`-symbol rows (stride = number of rows =
    /// `sf_len / BLOCK_LEN`), RS-corrects each row, and scatters corrected
    /// bytes back into place. Returns `(total_corrected_symbols,
    /// had_uncorrectable_row)`.
    fn rs_decode_superframe(&mut self) -> (u32, bool) {
        let sf_len = self.current.data.len();
        let subch_index = sf_len / rs::BLOCK_LEN;
        if subch_index == 0 {
            return (0, false);
        }

        let mut total_corrected = 0u32;
        let mut uncorrectable = false;

        for i in 0..subch_index {
            let mut rs_packet = [0u8; rs::BLOCK_LEN];
            for (pos, slot) in rs_packet.iter_mut().enumerate() {
                *slot = self.current.data[pos * subch_index + i];
            }

            match self.rs.decode(&mut rs_packet) {
                None => uncorrectable = true,
                Some(corrected_positions) => {
                    total_corrected += corrected_positions.len() as u32;
                    for loc in corrected_positions {
                        let Some(pos) = (loc as usize).checked_sub(rs::PAD) else {
                            continue;
                        };
                        self.current.data[pos * subch_index + i] = rs_packet[pos];
                    }
                }
            }
        }

        (total_corrected, uncorrectable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_zero_window_passes_firecode_by_construction() {
        // Documented above (and in `FrameSync::push`): this is exactly why
        // the frame scanner has to special-case an all-zero prefix rather
        // than relying on Firecode alone to reject it.
        assert!(check_firecode(&[0u8; 11]));
    }

    #[test]
    fn firecode_rejects_arbitrary_non_codeword_bytes() {
        let frame: [u8; 11] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        assert!(!check_firecode(&frame));
    }

    /// Finds the 2 check bytes (`frame[0..2]`) that make `check_firecode`
    /// pass for a given `frame[2..]` payload -- Firecode is a systematic
    /// 16-bit code over exactly those 2 bytes, so exactly one (b0, b1)
    /// solution exists for any payload (both of `check_firecode`'s final
    /// two update steps are bijections holding everything else fixed).
    /// Test-only: real broadcast frames carry these bytes already computed
    /// by the transmitter.
    ///
    /// On the rare payload whose unique solution happens to be `(0, 0)`
    /// (excluded by `FrameSync` itself, since that pair otherwise
    /// false-syncs on padding -- see `FrameSync::push`'s comment), nudges
    /// the payload's last byte and retries rather than returning a prefix
    /// `FrameSync` would refuse to recognize.
    fn solve_firecode_prefix(payload_from_byte2: &[u8]) -> Vec<u8> {
        let mut payload = payload_from_byte2.to_vec();
        for _attempt in 0..8 {
            let mut frame = vec![0u8, 0u8];
            frame.extend_from_slice(&payload);
            for b0 in 0..=255u16 {
                for b1 in 0..=255u16 {
                    frame[0] = b0 as u8;
                    frame[1] = b1 as u8;
                    if check_firecode(&frame) && !(b0 == 0 && b1 == 0) {
                        return frame;
                    }
                }
            }
            let last = payload.len() - 1;
            payload[last] = payload[last].wrapping_add(1);
        }
        panic!("no non-degenerate Firecode-valid prefix found after several payload perturbations");
    }

    #[test]
    fn frame_sync_finds_frame_after_leading_garbage() {
        let frame_size = 48; // e.g. a 16 kbit/s subchannel (16*3)
                             // `check_firecode` only ever reads `frame[0..11]` -- put
                             // non-degenerate content there (`header[2..11]`, once the 2 check
                             // bytes are prepended) and zero-fill the rest, which is irrelevant
                             // to Firecode and just needs to exist to reach `frame_size`.
        let mut header = vec![9u8, 10, 11, 12, 13, 14, 15, 16, 17];
        header.resize(frame_size - 2, 0);
        let good_frame = solve_firecode_prefix(&header);
        assert_eq!(good_frame.len(), frame_size);

        let mut stream = vec![0xAAu8; 17]; // misaligned leading garbage
        stream.extend_from_slice(&good_frame);
        stream.extend_from_slice(&[0u8; 5]); // trailing partial frame, buffered

        let mut sync = FrameSync::new(frame_size);
        let frames = sync.push(&stream);

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], good_frame);
        assert_eq!(sync.buffer.len(), 5);
    }

    #[test]
    fn decodes_one_superframe_end_to_end() {
        let bitrate_kbits: u16 = 8; // frame_size = 24, superframe_size = 110
        let frame_size = bitrate_kbits as usize * 3;
        let mut decoder = DabPlusDecoder::new(bitrate_kbits);
        let subch_index = (5 * frame_size) / rs::BLOCK_LEN; // 1 for this bitrate

        // Frame 0's header: dacRate=0, sbr=0, chanMode=0 (mono), ps=0 ->
        // numAUs=4, first AU start=8 (spec-fixed, not a test choice).
        let au_starts = [8u16, 35, 62, 89];
        let full_au_starts = [8u16, 35, 62, 89, 110u16];
        let mut superframe_data = vec![0u8; 5 * frame_size];

        // AAC config byte (frame[2]): dacRate=0,sbr=0,chanMode=0,ps=0,
        // mpegSurCfg=0 -> 0x00.
        superframe_data[2] = 0x00;
        // Pack au_starts[1..] (35, 62, 89) as 12-bit big-endian values into
        // frame[3..8].
        let pack = |a: u16, b: u16| -> [u8; 3] {
            [
                (a >> 4) as u8,
                (((a & 0x0F) << 4) | (b >> 8)) as u8,
                (b & 0xFF) as u8,
            ]
        };
        let p1 = pack(au_starts[1], au_starts[2]);
        superframe_data[3] = p1[0];
        superframe_data[4] = p1[1];
        superframe_data[5] = p1[2];
        // au_starts[3] (89) is a lone odd-index (i=3) entry: high byte +
        // high nibble only.
        superframe_data[6] = (au_starts[3] >> 4) as u8;
        superframe_data[7] = ((au_starts[3] & 0x0F) << 4) as u8;

        // AU payloads + CRC-CCITT trailers, per au_starts, ending at
        // superframe_size=110. Note AU0 starts at byte 8, inside frame 0
        // -- its first 3 bytes (8, 9, 10) fall *within* the 11-byte
        // Firecode-protected window too, so this has to happen before
        // solving for the Firecode check bytes below, not after.
        for w in 0..4 {
            let start = full_au_starts[w] as usize;
            let end = full_au_starts[w + 1] as usize;
            let payload_len = end - start - 2; // reserve 2 bytes for CRC
            for (k, b) in superframe_data[start..start + payload_len]
                .iter_mut()
                .enumerate()
            {
                *b = (w * 17 + k) as u8;
            }
            let crc = crc_ccitt_of(&superframe_data[start..start + payload_len]);
            superframe_data[start + payload_len] = (crc >> 8) as u8;
            superframe_data[start + payload_len + 1] = (crc & 0xFF) as u8;
        }

        // Solve for the 2 Firecode check bytes (frame[0..2]) now that
        // frame[2..24) -- including AU0's first 3 bytes -- is final.
        let header_tail = superframe_data[2..frame_size].to_vec();
        let solved = solve_firecode_prefix(&header_tail);
        assert_eq!(
            &solved[2..],
            &header_tail[..],
            "solver must not alter the payload it was given"
        );
        superframe_data[0] = solved[0];
        superframe_data[1] = solved[1];

        // RS-encode each interleaved row so the superframe is a valid RS
        // codeword throughout (decode() must not report uncorrectable).
        let rs_codec = RsCodec::new();
        for i in 0..subch_index {
            let mut data = [0u8; rs::DATA_LEN];
            for (pos, slot) in data.iter_mut().enumerate() {
                *slot = superframe_data[pos * subch_index + i];
            }
            let mut parity = [0u8; 10];
            rs_codec.encode(&data, &mut parity);
            for (pos, &p) in parity.iter().enumerate() {
                superframe_data[(rs::DATA_LEN + pos) * subch_index + i] = p;
            }
        }

        // `FrameSync`'s scan bound is strict `<` (matching the reference
        // exactly -- see `FrameSync::push`'s comment): a frame boundary is
        // only confirmed once at least one more byte has arrived past it,
        // so the last frame needs a trailing byte to be flushed out.
        superframe_data.push(0);

        let outputs = decoder.push_msc_data(&superframe_data);

        assert_eq!(outputs.len(), 1);
        let out = &outputs[0];
        assert_eq!(out.channels, 1);
        assert_eq!(out.sample_rate, 32000);
        assert!(!out.sbr_used);
        assert!(!out.ps_used);
        assert_eq!(out.ascty, ASCTY_DAB_PLUS);
        assert!(!out.rs_uncorrectable);
        assert_eq!(out.rs_corrected_symbols, 0);

        // All 4 AUs' payload bytes, concatenated (CRC stripped).
        let mut expected = Vec::new();
        for w in 0..4 {
            let len = (full_au_starts[w + 1] - full_au_starts[w]) as usize - 2;
            expected.extend((0..len).map(|k| (w * 17 + k) as u8));
        }
        assert_eq!(out.access_units, expected);

        // `access_unit_lengths` must let a caller recover exactly those
        // same 4 AUs' boundaries from the concatenated buffer.
        let expected_lengths: Vec<u16> = (0..4)
            .map(|w| full_au_starts[w + 1] - full_au_starts[w] - 2)
            .collect();
        assert_eq!(out.access_unit_lengths, expected_lengths);
        assert_eq!(
            out.access_unit_lengths
                .iter()
                .map(|&l| l as usize)
                .sum::<usize>(),
            out.access_units.len()
        );
    }

    /// Test-only CRC-CCITT computation matching `crate::tables::crc_ccitt_check`'s
    /// algorithm, used to construct a passing AU trailer.
    fn crc_ccitt_of(data: &[u8]) -> u16 {
        let mut crc: u16 = 0xffff;
        for &byte in data {
            crc = (crc << 8) ^ crate::tables::CRC_CCITT_TABLE[usize::from((crc >> 8) as u8 ^ byte)];
        }
        crc ^ 0xffff
    }
}
