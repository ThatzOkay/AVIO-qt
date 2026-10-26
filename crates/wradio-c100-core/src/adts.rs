//! ADTS (Audio Data Transport Stream) framing for this crate's raw AAC
//! access units (`DabPlusOutput::access_units`/`access_unit_lengths`,
//! `dabplus.rs`).
//!
//! DAB+ carries raw `raw_data_block`s with no container framing of their
//! own -- neither `dabplus.rs` nor the reference need one, since AAC
//! syntax is self-delimiting and a decoder that parses it directly (this
//! crate's own eventual native playback path included) doesn't need
//! external frame boundaries. This module exists purely for *external*
//! consumers that expect a standard container instead: originally just
//! `examples/scan_and_listen.rs`'s `--dump-aac` (for `ffmpeg`/`ffplay`,
//! neither of which parses bare `raw_data_block`s), and now also
//! `tauri-plugin-wradio-c100`'s native playback path, whose AAC decoder
//! (`syom`) explicitly documents itself as expecting ADTS- or LATM/LOAS-
//! framed input, not raw access units either. Factored out here (rather
//! than left duplicated a second time) so both stay byte-for-byte
//! identical rather than risking silent drift.

/// Standard MPEG-4 `samplingFrequencyIndex` table (ADTS/ASC), the subset
/// this crate's two possible AAC core rates (16000/24000, when
/// `sbr_used`, being the pre-SBR-upsample half of `sample_rate`; else
/// `sample_rate` itself directly -- see `dabplus.rs`'s superframe header
/// comment on `sample_rate` meaning the *output*, post-SBR rate) can map
/// to.
pub const SAMPLE_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

/// Looks up `core_rate_hz`'s ADTS `sampling_frequency_index`, `None` if
/// it's not one of the 13 standard MPEG-4 rates.
pub fn sampling_frequency_index(core_rate_hz: u32) -> Option<u8> {
    SAMPLE_RATES.iter().position(|&r| r == core_rate_hz).map(|i| i as u8)
}

/// Writes one 7-byte ADTS header (no CRC) for an AAC-LC raw_data_block of
/// `payload_len` bytes, implicit-signaling any SBR/PS extension already
/// embedded in the DAB+ raw_data_block itself (which is how a compliant
/// decoder, e.g. ffmpeg's native AAC decoder or `syom`, recovers HE-AAC v2
/// from a plain AAC-LC-profile ADTS stream -- DAB+ carries HE-AACv2
/// exactly this way, per ETS 102 563).
pub fn write_header(out: &mut Vec<u8>, payload_len: usize, sampling_frequency_index: u8, channel_config: u8) {
    let frame_length = (7 + payload_len) as u16;
    const AAC_LC_PROFILE: u8 = 1; // ObjectType(2, AAC LC) - 1
    const BUFFER_FULLNESS: u16 = 0x7FF; // VBR / unspecified

    out.push(0xFF);
    out.push(0xF1); // MPEG-4, layer 00, protection_absent=1 (no CRC)
    out.push((AAC_LC_PROFILE << 6) | (sampling_frequency_index << 2) | ((channel_config >> 2) & 0x01));
    out.push(((channel_config & 0x03) << 6) | ((frame_length >> 11) as u8 & 0x03));
    out.push(((frame_length >> 3) & 0xFF) as u8);
    out.push((((frame_length & 0x07) as u8) << 5) | ((BUFFER_FULLNESS >> 6) as u8 & 0x1F));
    out.push(((BUFFER_FULLNESS & 0x3F) as u8) << 2);
}

/// ADTS-wraps every access unit in `output` (using
/// [`DabPlusOutput::access_unit_lengths`](crate::dabplus::DabPlusOutput::access_unit_lengths)
/// to recover each AU's boundaries), appending each resulting frame to
/// `out` in order. A superframe whose core rate has no ADTS
/// `sampling_frequency_index` (shouldn't happen for any rate this crate
/// actually produces, see [`sampling_frequency_index`]) is silently
/// skipped rather than panicking -- matches `scan_and_listen.rs`'s own
/// prior handling of that case.
pub fn wrap_superframe(out: &mut Vec<u8>, output: &crate::dabplus::DabPlusOutput) {
    let core_rate = if output.sbr_used {
        output.sample_rate / 2
    } else {
        output.sample_rate
    };
    let Some(freq_idx) = sampling_frequency_index(core_rate) else {
        return;
    };
    let mut pos = 0usize;
    for &len in &output.access_unit_lengths {
        let len = len as usize;
        let au = &output.access_units[pos..pos + len];
        pos += len;
        write_header(out, len, freq_idx, output.channels);
        out.extend_from_slice(au);
    }
}
