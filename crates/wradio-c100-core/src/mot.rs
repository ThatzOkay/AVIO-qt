//! MOT (Multimedia Object Transfer, ETSI EN 301 234) data-group decode:
//! reassembles MOT header + body datagroups (keyed by transport id) into
//! complete objects. Ported from `motdecoder.cpp/h`.
//!
//! Simplified from the reference per the same pattern used for `pad.rs`/
//! `dls.rs`: no `DabMotUserapplicationDecoder` registration list -- MOT
//! slideshow (`slideshow.rs`) is the only MOT consumer in scope (per
//! `RUST_PORT_PROMPT.md`), so a completed `IMAGE`-content object is simply
//! returned to the caller to hand to `slideshow.rs` directly, rather than
//! dispatched through a registry of decoders.

use std::collections::HashMap;

use crate::tables::crc_ccitt_check;

/// MOT content type, ETSI TS 101 756 table 17 (`MOT_CONTENT_TYPE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotContentType {
    GeneralData,
    Text,
    Image,
    Audio,
    Video,
    MotTransport,
    System,
    Application,
    Proprietary,
    Reserved(u8),
}

impl MotContentType {
    fn from_code(code: u8) -> Self {
        match code {
            0 => Self::GeneralData,
            1 => Self::Text,
            2 => Self::Image,
            3 => Self::Audio,
            4 => Self::Video,
            5 => Self::MotTransport,
            6 => Self::System,
            7 => Self::Application,
            8 => Self::Proprietary,
            other => Self::Reserved(other),
        }
    }
}

/// One in-progress or completed MOT object. Mirrors `MOT_Data`.
#[derive(Debug, Clone)]
pub struct MotData {
    pub transport_id: u16,
    pub content_type: MotContentType,
    pub content_subtype: u16,
    /// Header extension parameters, keyed by parameter id (ETSI TS 101 756
    /// table 17), raw bytes.
    pub header_params: HashMap<u8, Vec<u8>>,
    pub body_data: Vec<u8>,
    pub body_size: u32,
    /// `None` mirrors the reference's `previousSegmentNumber{-1}` (no body
    /// segment received yet).
    previous_segment_number: Option<u16>,
}

impl MotData {
    fn new(transport_id: u16) -> Self {
        Self {
            transport_id,
            content_type: MotContentType::GeneralData,
            content_subtype: 0,
            header_params: HashMap::new(),
            body_data: Vec::new(),
            body_size: 0,
            previous_segment_number: None,
        }
    }
}

/// MOT data-group reassembly state. Mirrors `MotDecoder`.
#[derive(Debug, Default)]
pub struct MotDecoder {
    headers: HashMap<u16, MotData>,
}

impl MotDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// `MotDecoder::motDataInput`. Returns a completed `IMAGE`-content
    /// object once its body is fully reassembled -- the only content type
    /// the reference actually acts on (others are only logged as
    /// "unhandled"), so that's the only case this returns `Some` for too.
    pub fn mot_data_input(&mut self, msc_datagroup: &[u8]) -> Option<MotData> {
        let mut pos = 0usize;

        let &b0 = msc_datagroup.first()?;
        let extension_flag = (b0 & 0x80) != 0;
        let crc_flag = (b0 & 0x40) != 0;

        if crc_flag && !crc_ccitt_check(msc_datagroup) {
            return None;
        }

        let segment_flag = (b0 & 0x20) != 0;
        let user_access_flag = (b0 & 0x10) != 0;
        let datagroup_type = b0 & 0x0F;
        pos += 1;

        // continuityIdx/repetitionIdx: parsed by the reference but never
        // consumed downstream, matching that we don't need them either.
        pos += 1;

        if extension_flag {
            pos += 2;
        }

        let mut is_last = false;
        let mut seg_num: u16 = 0xFFFF;
        if segment_flag {
            let &b = msc_datagroup.get(pos)?;
            pos += 1;
            let &lo = msc_datagroup.get(pos)?;
            pos += 1;
            is_last = (b & 0x80) != 0;
            seg_num = (((b & 0x7F) as u16) << 8) | lo as u16;
        }

        let mut transport_id: u16 = 0xFFFF;
        if user_access_flag {
            let &b = msc_datagroup.get(pos)?;
            pos += 1;
            let transport_id_flag = (b & 0x10) != 0;
            let length_indicator = b & 0x0F;

            if transport_id_flag {
                let &hi = msc_datagroup.get(pos)?;
                pos += 1;
                let &lo = msc_datagroup.get(pos)?;
                pos += 1;
                transport_id = ((hi as u16) << 8) | lo as u16;
            }

            // End user address bytes: parsed by the reference but never
            // consumed downstream either -- skip past them.
            pos = (pos + length_indicator.saturating_sub(2) as usize).min(msc_datagroup.len());
        }

        // Segmentation header.
        let &b = msc_datagroup.get(pos)?;
        pos += 1;
        let &seg_size_lo = msc_datagroup.get(pos)?;
        pos += 1;
        let segment_size = (((b & 0x1F) as u16) << 8) | seg_size_lo as u16;

        match datagroup_type {
            3 => {
                // MOT_HEADER
                let mut mot = MotData::new(transport_id);

                // Header Core (7 bytes): 28-bit body size, then a
                // (computed-but-unused, see below) header size, then
                // content type/subtype.
                let &hb0 = msc_datagroup.get(pos)?;
                pos += 1;
                let &hb1 = msc_datagroup.get(pos)?;
                pos += 1;
                let &hb2 = msc_datagroup.get(pos)?;
                pos += 1;
                let &hb3 = msc_datagroup.get(pos)?;
                pos += 1;
                mot.body_size = ((hb0 as u32) << 20)
                    | ((hb1 as u32) << 12)
                    | ((hb2 as u32) << 4)
                    | ((hb3 as u32 & 0xF0) >> 4);

                // motHeaderSize is computed by the reference but never
                // actually used to bound anything (the header-extension
                // loop below runs to the datagroup's own end instead) --
                // not computed here either. Its own third term is also a
                // reference bug (`(*dgIter & 0x0F) >> 7`, always zero from
                // that mask/shift order) that would be moot regardless.
                let &_hb4 = msc_datagroup.get(pos)?;
                pos += 1;

                let &hb5 = msc_datagroup.get(pos)?;
                mot.content_type = MotContentType::from_code((hb5 & 0x7E) >> 1);
                pos += 1;
                let &hb6 = msc_datagroup.get(pos)?;
                pos += 1;
                mot.content_subtype = (((hb5 & 0x01) as u16) << 8) | hb6 as u16;

                // Header extension parameters, running to the end of the
                // datagroup (minus its trailing CRC, if present).
                let end_bound = msc_datagroup
                    .len()
                    .saturating_sub(if crc_flag { 2 } else { 0 });
                while pos < end_bound {
                    let Some(&pb) = msc_datagroup.get(pos) else {
                        break;
                    };
                    let length_indicator = (pb & 0xC0) >> 6;
                    let param_id = pb & 0x3F;
                    pos += 1;

                    let param_length: usize = match length_indicator {
                        1 => 1,
                        2 => 4,
                        3 => {
                            let Some(&lb) = msc_datagroup.get(pos) else {
                                break;
                            };
                            if (lb & 0x80) != 0 {
                                let hi = (lb & 0x7F) as usize;
                                pos += 1;
                                let Some(&lo) = msc_datagroup.get(pos) else {
                                    break;
                                };
                                pos += 1;
                                (hi << 8) | lo as usize
                            } else {
                                let v = (lb & 0x7F) as usize;
                                pos += 1;
                                v
                            }
                        }
                        _ => 0, // 0: no data; other: RFU, treated as 0-length
                    };

                    let end = (pos + param_length).min(msc_datagroup.len());
                    mot.header_params
                        .insert(param_id, msc_datagroup[pos..end].to_vec());
                    pos = end;
                }

                // `std::map::insert` is a no-op when the key already
                // exists (unlike a blind overwrite) -- matches: a repeated
                // MOT_HEADER for a transport id already in progress is
                // ignored, not replaced.
                self.headers.entry(transport_id).or_insert(mot);
                None
            }
            4 => {
                // MOT_BODY_UNSCRAMBLED
                let mot = self.headers.get_mut(&transport_id)?;
                let expected = mot.previous_segment_number.map_or(0, |p| p + 1);

                if seg_num != expected {
                    self.headers.remove(&transport_id);
                    return None;
                }

                mot.previous_segment_number = Some(seg_num);
                let end = (pos + segment_size as usize).min(msc_datagroup.len());
                mot.body_data.extend_from_slice(&msc_datagroup[pos..end]);

                let mut completed = None;
                if mot.body_data.len() as u32 == mot.body_size
                    && mot.content_type == MotContentType::Image
                {
                    completed = Some(mot.clone());
                }

                if is_last {
                    self.headers.remove(&transport_id);
                }

                completed
            }
            _ => None,
        }
    }
}
