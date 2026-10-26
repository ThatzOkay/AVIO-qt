//! X-PAD (extended Programme-Associated Data) framing: splits raw PAD
//! bytes (one AU's `pad_units` entry, see
//! [`crate::dabplus::DabPlusOutput`]) into F-PAD + X-PAD subfields and
//! dispatches Dynamic Label ([`crate::dls`]) updates and MOT data groups.
//! Ported from `paddecoder.cpp/h`.
//!
//! Simplified from the reference per `RUST_PORT_PROMPT.md`'s explicit
//! guidance: dispatches straight to [`crate::dls::DlsDecoder`] by matching
//! the X-PAD application type, instead of the reference's
//! `DabUserApplication`/`DabUserapplicationDecoder` type-registry lookup
//! (`m_userApps`/`m_userAppDecoders`) -- that lookup only ever logs in the
//! reference anyway, nothing functional is lost by dropping it.
//!
//! **Byte order note**: per the DAB PAD channel format, F-PAD occupies the
//! *last* 2 bytes of the PAD field, and any X-PAD data preceding it is
//! read backwards from there towards the start -- the reference does this
//! with a `reverse_iterator`. This module reverses the whole slice once up
//! front instead and walks it with a plain forward cursor from there,
//! which produces byte-for-byte the same subfields (`reversed[i] ==
//! pad_data[pad_data.len() - 1 - i]`, so a forward `k`-byte read at cursor
//! position `i` yields exactly the reference's `*(padRiter+j)` for
//! `j in 0..k`) without needing a reverse-iterator equivalent in Rust.

use crate::dls::{DlsDecoder, DlsUpdate};
use crate::mot::MotDecoder;
use crate::slideshow::{self, SlideshowUpdate};
use crate::tables::crc_ccitt_check;

/// `PadDecoder::XPAD_SIZE`.
const XPAD_SIZE: [u8; 8] = [4, 6, 8, 12, 16, 24, 32, 48];

// PAD_APPLICATION_TYPE (global_definitions.h) values actually acted on;
// everything else (including 4-11/16-30 "user defined", CA message
// start/continuation, and NOT_USED=31) is a no-op here, matching the
// reference (which only ever logs those, see this module's doc comment).
const APP_DATA_GROUP_LENGTH_INDICATOR: u8 = 1;
const APP_DLS_DATAGROUP_START: u8 = 2;
const APP_DLS_DATAGROUP_CONTINUATION: u8 = 3;
const APP_MOT_DATAGROUP_START: u8 = 12;
const APP_MOT_DATAGROUP_CONTINUATION: u8 = 13;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XPadIndicator {
    NoXpad,
    ShortXpad,
    VariableXpad,
    Rfu,
}

impl XPadIndicator {
    fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::NoXpad,
            1 => Self::ShortXpad,
            2 => Self::VariableXpad,
            _ => Self::Rfu,
        }
    }
}

/// Output of one [`PadDecoder::pad_data_input`] call.
#[derive(Debug, Default)]
pub struct PadOutput {
    pub dls_update: Option<DlsUpdate>,
    pub slideshow_updates: Vec<SlideshowUpdate>,
}

/// X-PAD framing state. Mirrors `PadDecoder`, including its ownership of a
/// `MotDecoder` (`m_motDecoder`) -- completed MOT data groups are routed
/// through it (and, for `IMAGE` content, through `slideshow::decode`)
/// automatically rather than left for the caller to wire up.
pub struct PadDecoder {
    /// Mirrors `m_currentDataGroupLength`/`m_currentDataGroup`.
    current_data_group_length: u16,
    current_data_group: Vec<u8>,
    /// Mirrors `m_noCiLastLength`/`m_noCiLastXpAdAppType`/
    /// `m_noCiLastShortXpAdAppType`; the latter two use `0xFF` as the
    /// reference's own "none yet" sentinel.
    no_ci_last_length: u8,
    no_ci_last_xpad_app_type: u8,
    no_ci_last_short_xpad_app_type: u8,
    dls: DlsDecoder,
    mot: MotDecoder,
}

impl Default for PadDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl PadDecoder {
    pub fn new() -> Self {
        Self {
            current_data_group_length: 0,
            current_data_group: Vec::new(),
            no_ci_last_length: 0,
            no_ci_last_xpad_app_type: 0xFF,
            no_ci_last_short_xpad_app_type: 0xFF,
            dls: DlsDecoder::new(),
            mot: MotDecoder::new(),
        }
    }

    /// `PadDecoder::reset`: resets the Dynamic Label decoder, matching the
    /// reference (which resets every registered `DabUserapplicationDecoder`
    /// -- the only one wired up here is DLS). In-progress MOT data-group
    /// state isn't cleared by the reference's `reset()` either.
    pub fn reset(&mut self) {
        self.dls.reset();
    }

    /// `PadDecoder::padDataInput`.
    pub fn pad_data_input(&mut self, pad_data: &[u8]) -> PadOutput {
        let mut output = PadOutput::default();
        if pad_data.len() < 2 {
            return output;
        }

        // See this module's doc comment for why reversing once up front is
        // equivalent to the reference's reverse_iterator walk.
        let reversed: Vec<u8> = pad_data.iter().rev().copied().collect();

        // F-PAD: byte L (CI flag, Z flag -- Z is only ever logged by the
        // reference, not surfaced here) then byte L-1 (F-PAD type, X-PAD
        // indicator).
        let ci_present = (reversed[0] & 0x02) != 0;
        let f_pad_type = (reversed[1] & 0xC0) >> 6;
        let xpad_indicator = XPadIndicator::from_bits((reversed[1] & 0x30) >> 4);
        let mut pos = 2usize;

        if f_pad_type != 0 {
            // Only F-PAD type 0 carries X-PAD subfields.
            return output;
        }

        let mut apps: Vec<(u8, u8)> = Vec::new(); // (xpad_size, app_type)

        match xpad_indicator {
            XPadIndicator::NoXpad | XPadIndicator::Rfu => {}
            XPadIndicator::ShortXpad => {
                let (xpad_size, app_type);
                if ci_present {
                    let Some(&b) = reversed.get(pos) else {
                        return output;
                    };
                    pos += 1;
                    let app = b & 0x1F;
                    self.no_ci_last_short_xpad_app_type = app;
                    app_type = app;
                    xpad_size = 3;
                } else {
                    app_type = self.no_ci_last_short_xpad_app_type.wrapping_add(1);
                    xpad_size = 4;
                }
                apps.push((xpad_size, app_type));
            }
            XPadIndicator::VariableXpad => {
                if ci_present {
                    for _ in 0..4 {
                        let Some(&b) = reversed.get(pos) else { break };
                        pos += 1;
                        let xpad_length_index = (b & 0xE0) >> 5;
                        let xpad_app_type = b & 0x1F;

                        if xpad_app_type == 0 {
                            break;
                        }

                        let xpad_size = XPAD_SIZE[xpad_length_index as usize];
                        self.no_ci_last_length = xpad_size;
                        self.no_ci_last_xpad_app_type = xpad_app_type;
                        apps.push((xpad_size, xpad_app_type));
                    }
                } else if self.no_ci_last_length > 0 || self.no_ci_last_xpad_app_type != 0xFF {
                    // Not a continuation of the last X-PAD app type: bump
                    // to the odd (data) half of the type pair.
                    if self.no_ci_last_xpad_app_type.is_multiple_of(2) {
                        self.no_ci_last_xpad_app_type =
                            self.no_ci_last_xpad_app_type.wrapping_add(1);
                    }
                    apps.push(((pad_data.len() - 2) as u8, self.no_ci_last_xpad_app_type));
                }
            }
        }

        for (size, app_type) in apps {
            let size = size as usize;
            let Some(subfield) = reversed.get(pos..pos + size) else {
                break;
            };
            pos += size;

            match app_type {
                APP_DATA_GROUP_LENGTH_INDICATOR => {
                    if crc_ccitt_check(subfield) {
                        if !self.current_data_group.is_empty() {
                            let completed = std::mem::take(&mut self.current_data_group);
                            if let Some(image) = self.mot.mot_data_input(&completed) {
                                output.slideshow_updates.push(slideshow::decode(&image));
                            }
                        }
                        self.current_data_group_length =
                            (((subfield[0] & 0x3F) as u16) << 8) | subfield[1] as u16;
                    } else {
                        self.current_data_group.clear();
                        self.current_data_group_length = 0;
                    }
                }
                APP_MOT_DATAGROUP_START | APP_MOT_DATAGROUP_CONTINUATION => {
                    if app_type == APP_MOT_DATAGROUP_CONTINUATION {
                        let remaining = (self.current_data_group_length as usize)
                            .saturating_sub(self.current_data_group.len());
                        if subfield.len() <= remaining {
                            self.current_data_group.extend_from_slice(subfield);
                        } else {
                            self.current_data_group
                                .extend_from_slice(&subfield[..remaining]);
                        }
                    } else {
                        self.current_data_group.extend_from_slice(subfield);
                    }
                }
                APP_DLS_DATAGROUP_START | APP_DLS_DATAGROUP_CONTINUATION => {
                    if let Some(update) = self.dls.application_data_input(subfield, app_type) {
                        output.dls_update = Some(update);
                    }
                }
                _ => {}
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registered_tables::CharacterSet;

    fn crc_ccitt_of(data: &[u8]) -> u16 {
        let mut crc: u16 = 0xffff;
        for &byte in data {
            crc = (crc << 8) ^ crate::tables::CRC_CCITT_TABLE[usize::from((crc >> 8) as u8 ^ byte)];
        }
        crc ^ 0xffff
    }

    /// Builds one on-air PAD field (forward byte order, ready to hand to
    /// [`PadDecoder::pad_data_input`]) carrying a single variable-X-PAD,
    /// CI-present DLS subfield -- `dls_subfield` padded up to
    /// `XPAD_SIZE[length_index]` bytes (variable X-PAD subfields are
    /// fixed-size on air; real DLS data shorter than that is followed by
    /// don't-care padding, which the DLS length/CRC fields account for).
    fn build_variable_xpad_dls_field(dls_subfield: &[u8], length_index: u8) -> Vec<u8> {
        let size = XPAD_SIZE[length_index as usize] as usize;
        assert!(dls_subfield.len() <= size);
        let mut data = dls_subfield.to_vec();
        data.resize(size, 0);

        // Build in *read* order (i.e. what `reversed[]` inside
        // `pad_data_input` will reconstruct), then reverse once to get the
        // actual forward-order PAD field bytes -- see this module's own
        // doc comment on why that double-reversal is correct.
        let mut read_order = vec![
            0x02, // byte L: CI present (bit 1)
            // byte L-1: fPadType=0 (bits 7-6), X-PAD indicator = VARIABLE (0b10, bits 5-4)
            0x20,
            // Indicator 1: length_index (bits 7-5) + appType=DLS_DATAGROUP_START=2 (bits 4-0)
            (length_index << 5) | APP_DLS_DATAGROUP_START,
            // Indicator 2: appType=0 (END_MARKER) -- stops the up-to-4 scan.
            0x00,
        ];
        read_order.extend_from_slice(&data);

        read_order.into_iter().rev().collect()
    }

    #[test]
    fn dls_one_and_only_segment_via_variable_xpad() {
        // DLS ONE_AND_ONLY segment: byte0 = toggle(0)+segType(ONE_AND_ONLY=0b11<<5)+cmdFlag(0)+length_lo.
        let text = b"HELLO";
        let length_field = (text.len() - 1) as u8;
        let seg_type_bits = 0b11u8; // ONE_AND_ONLY
        let byte0 = (seg_type_bits << 5) | length_field;
        let byte1 = 0x00; // charset=EBU Latin(0), rfa=0

        let mut dls_bytes = vec![byte0, byte1];
        dls_bytes.extend_from_slice(text);
        let crc = crc_ccitt_of(&dls_bytes);
        dls_bytes.push((crc >> 8) as u8);
        dls_bytes.push(crc as u8);
        assert_eq!(dls_bytes.len(), 4 + text.len()); // 9 bytes -- fits XPAD_SIZE[3]=12

        let pad_field = build_variable_xpad_dls_field(&dls_bytes, 3);

        let mut decoder = PadDecoder::new();

        // A DLS_DATAGROUP_START only *processes* whatever was accumulated
        // from an *earlier* start -- see `DlsDecoder::application_data_input`'s
        // doc comment -- so the first call just stores this segment.
        let first = decoder.pad_data_input(&pad_field);
        assert!(first.dls_update.is_none());

        // A second start (the same field again, for simplicity) triggers
        // processing of the one stored above.
        let second = decoder.pad_data_input(&pad_field);
        let update = second
            .dls_update
            .expect("DLS update expected on the second call");
        assert_eq!(update.text, "HELLO");
        assert_eq!(update.charset, CharacterSet::EbuLatin);
        assert!(update.plus_tags.is_empty());
    }
}
