//! Dynamic Label (DLS) + DL+ decode -- the DAB equivalent of RDS RadioText
//! / RT+: the live "now playing" text (station slogan, artist/title, ...)
//! carried as X-PAD, plus optional DL+ tags mapping substrings of that
//! text to a content type. Ported from `dynamiclabeldecoder.cpp/h`.
//!
//! Simplified from the reference per `RUST_PORT_PROMPT.md`'s explicit
//! guidance: no `DabUserapplicationDecoder`/plugin-registration machinery
//! (that's `pad.rs`'s job, dispatching straight to [`DlsDecoder`] by
//! matching `PadApplicationType` instead of a type-registry lookup) -- but
//! the DLS segment/charset/DL+ parsing itself stays faithful to the
//! reference.

use crate::fic::fig::decode_label_text;
use crate::registered_tables::CharacterSet;
use crate::tables::crc_ccitt_check;

/// DL+ content type, ETSI TS 102 980 table 9 (`DL_PLUS_CONTENT_TYPE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DlPlusContentType {
    Dummy,
    ItemTitle,
    ItemAlbum,
    ItemTrackNumber,
    ItemArtist,
    ItemComposition,
    ItemMovement,
    ItemConductor,
    ItemComposer,
    ItemBand,
    ItemComment,
    ItemGenre,
    InfoNews,
    InfoNewsLocal,
    InfoStockmarket,
    InfoSport,
    InfoLottery,
    InfoHoroscope,
    InfoDailyDiversion,
    InfoHealth,
    InfoEvent,
    InfoScene,
    InfoCinema,
    InfoTv,
    InfoDateTime,
    InfoWeather,
    InfoTraffic,
    InfoAlarm,
    InfoAdvertisement,
    InfoUrl,
    InfoOther,
    StationnameShort,
    StationnameLong,
    ProgrammeNow,
    ProgrammeNext,
    ProgrammePart,
    ProgrammeHost,
    ProgrammeEditorialStaff,
    ProgrammeFrequency,
    ProgrammeHomepage,
    ProgrammeSubchannel,
    PhoneHotline,
    PhoneStudio,
    PhoneOther,
    SmsStudio,
    SmsOther,
    EmailHotline,
    EmailStudio,
    EmailOther,
    MmsOther,
    Chat,
    ChatCenter,
    VoteQuestion,
    VoteCentre,
    Rfu(u8),
    PrivateClass1,
    PrivateClass2,
    PrivateClass3,
    DescriptorPlace,
    DescriptorAppointment,
    DescriptorIdentifier,
    DescriptorPurchase,
    DescriptorGetData,
    /// Any 7-bit code the table above doesn't cover (RFU beyond the two
    /// named `Rfu` slots, or a future/unassigned value).
    Reserved(u8),
}

impl DlPlusContentType {
    /// `code` is the 7-bit content type field (already masked to `& 0x7F`
    /// by the caller).
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Dummy,
            1 => Self::ItemTitle,
            2 => Self::ItemAlbum,
            3 => Self::ItemTrackNumber,
            4 => Self::ItemArtist,
            5 => Self::ItemComposition,
            6 => Self::ItemMovement,
            7 => Self::ItemConductor,
            8 => Self::ItemComposer,
            9 => Self::ItemBand,
            10 => Self::ItemComment,
            11 => Self::ItemGenre,
            12 => Self::InfoNews,
            13 => Self::InfoNewsLocal,
            14 => Self::InfoStockmarket,
            15 => Self::InfoSport,
            16 => Self::InfoLottery,
            17 => Self::InfoHoroscope,
            18 => Self::InfoDailyDiversion,
            19 => Self::InfoHealth,
            20 => Self::InfoEvent,
            21 => Self::InfoScene,
            22 => Self::InfoCinema,
            23 => Self::InfoTv,
            24 => Self::InfoDateTime,
            25 => Self::InfoWeather,
            26 => Self::InfoTraffic,
            27 => Self::InfoAlarm,
            28 => Self::InfoAdvertisement,
            29 => Self::InfoUrl,
            30 => Self::InfoOther,
            31 => Self::StationnameShort,
            32 => Self::StationnameLong,
            33 => Self::ProgrammeNow,
            34 => Self::ProgrammeNext,
            35 => Self::ProgrammePart,
            36 => Self::ProgrammeHost,
            37 => Self::ProgrammeEditorialStaff,
            38 => Self::ProgrammeFrequency,
            39 => Self::ProgrammeHomepage,
            40 => Self::ProgrammeSubchannel,
            41 => Self::PhoneHotline,
            42 => Self::PhoneStudio,
            43 => Self::PhoneOther,
            44 => Self::SmsStudio,
            45 => Self::SmsOther,
            46 => Self::EmailHotline,
            47 => Self::EmailStudio,
            48 => Self::EmailOther,
            49 => Self::MmsOther,
            50 => Self::Chat,
            51 => Self::ChatCenter,
            52 => Self::VoteQuestion,
            53 => Self::VoteCentre,
            54 | 55 => Self::Rfu(code),
            56 => Self::PrivateClass1,
            57 => Self::PrivateClass2,
            58 => Self::PrivateClass3,
            59 => Self::DescriptorPlace,
            60 => Self::DescriptorAppointment,
            61 => Self::DescriptorIdentifier,
            62 => Self::DescriptorPurchase,
            63 => Self::DescriptorGetData,
            other => Self::Reserved(other),
        }
    }
}

/// One DL+ tag: a content type plus the substring of the label's text it
/// applies to.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DlPlusTag {
    pub content_type: DlPlusContentType,
    /// Empty for `content_type == Dummy` (the reference leaves the string
    /// unset in that case too, but still emits the tag).
    pub text: String,
}

/// One completed Dynamic Label update.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DlsUpdate {
    pub text: String,
    pub charset: CharacterSet,
    pub item_toggle: bool,
    pub item_running: bool,
    pub plus_tags: Vec<DlPlusTag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentType {
    Intermediate,
    Last,
    First,
    OneAndOnly,
}

impl SegmentType {
    fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::Intermediate,
            1 => Self::Last,
            2 => Self::First,
            _ => Self::OneAndOnly,
        }
    }
}

/// Dynamic Label decode state. Mirrors `DynamiclabelDecoder`.
#[derive(Debug)]
pub struct DlsDecoder {
    /// The X-PAD subfield bytes accumulated since the last
    /// `DlsDatagroupStart`, still including its own header/CRC -- mirrors
    /// `m_dlsData`.
    dls_data: Vec<u8>,
    /// The DLS text bytes assembled across a First/Intermediate*/Last
    /// segment run, still charset-undecoded -- mirrors `m_dlsFullData`.
    dls_full_data: Vec<u8>,
    dls_full_seg_num: u8,
    current_charset: CharacterSet,
    is_dynamic_plus: bool,
}

impl Default for DlsDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl DlsDecoder {
    pub fn new() -> Self {
        Self {
            dls_data: Vec::new(),
            dls_full_data: Vec::new(),
            dls_full_seg_num: 0,
            current_charset: CharacterSet::Other(0xFF),
            is_dynamic_plus: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// `DynamiclabelDecoder::applicationDataInput`. `data_type` is 2 for a
    /// DLS datagroup start, 3 for a continuation (matching
    /// `PAD_APPLICATION_TYPE`); any other value is a no-op, matching the
    /// reference's two disjoint `if`s.
    pub fn application_data_input(&mut self, app_data: &[u8], data_type: u8) -> Option<DlsUpdate> {
        let mut update = None;

        if data_type == 2 {
            // A new segment starting means whatever was accumulated in
            // `dls_data` since the *previous* start is now complete --
            // process it before resetting for the new one.
            if self.dls_data.len() >= 4 {
                update = self.process_complete_segment();
            }

            self.dls_data.clear();
            self.dls_data.extend_from_slice(app_data);
        } else if data_type == 3 && !self.dls_data.is_empty() {
            self.dls_data.extend_from_slice(app_data);
        }

        update
    }

    fn process_complete_segment(&mut self) -> Option<DlsUpdate> {
        let byte0 = self.dls_data[0];
        let seg_type = SegmentType::from_bits((byte0 & 0x60) >> 5);
        let command_flag = (byte0 & 0x10) != 0;

        if !command_flag {
            self.process_char_data_segment(seg_type)
        } else {
            self.process_command_segment(seg_type)
        }
    }

    fn process_char_data_segment(&mut self, seg_type: SegmentType) -> Option<DlsUpdate> {
        let length = (self.dls_data[0] & 0x0F) + 1;

        if !crc_ccitt_check(&self.dls_data[..(length as usize + 4).min(self.dls_data.len())]) {
            self.dls_data.clear();
            return None;
        }

        // Past the 1-byte segment header.
        let mut pos = 1usize;
        let mut update = None;

        match seg_type {
            SegmentType::First => {
                let &header = self.dls_data.get(pos)?;
                self.current_charset = CharacterSet::from_code((header & 0xF0) >> 4);
                pos += 1;

                self.dls_full_data.clear();
                self.dls_full_seg_num = 0;
                self.extend_full_data(pos, length as usize);
            }
            SegmentType::Intermediate | SegmentType::Last => {
                let &header = self.dls_data.get(pos)?;
                let seg_num = (header & 0x70) >> 4;
                pos += 1;

                if !self.dls_full_data.is_empty() {
                    if seg_num != self.dls_full_seg_num + 1 {
                        self.dls_full_data.clear();
                        self.dls_full_seg_num = 0;
                        return None;
                    }
                    self.dls_full_seg_num = seg_num;
                    self.extend_full_data(pos, length as usize);

                    if seg_type == SegmentType::Last && !self.is_dynamic_plus {
                        update = Some(DlsUpdate {
                            text: decode_label_text(&self.dls_full_data, self.current_charset),
                            charset: self.current_charset,
                            item_toggle: false,
                            item_running: false,
                            plus_tags: Vec::new(),
                        });
                    }
                }
            }
            SegmentType::OneAndOnly => {
                let &header = self.dls_data.get(pos)?;
                self.current_charset = CharacterSet::from_code((header & 0xF0) >> 4);
                pos += 1;

                self.dls_full_data.clear();
                self.extend_full_data(pos, length as usize);
                self.dls_full_seg_num = 0;

                if !self.is_dynamic_plus {
                    update = Some(DlsUpdate {
                        text: decode_label_text(&self.dls_full_data, self.current_charset),
                        charset: self.current_charset,
                        item_toggle: false,
                        item_running: false,
                        plus_tags: Vec::new(),
                    });
                }
            }
        }

        self.dls_data.clear();
        update
    }

    /// Appends `dls_data[start..start+length]` (clamped to what's
    /// actually present) to `dls_full_data`.
    fn extend_full_data(&mut self, start: usize, length: usize) {
        let end = (start + length).min(self.dls_data.len());
        if start < end {
            self.dls_full_data
                .extend_from_slice(&self.dls_data[start..end]);
        }
    }

    fn process_command_segment(&mut self, seg_type: SegmentType) -> Option<DlsUpdate> {
        let mut data = std::mem::take(&mut self.dls_data);
        remove_trailing_zero_padding(&mut data);

        if !crc_ccitt_check(&data) {
            return None;
        }

        let mut pos = 0usize;
        let &byte0 = data.get(pos)?;
        let command = byte0 & 0x0F;
        pos += 1;

        match command {
            // CLEAR_DISPLAY: not actually implemented by the reference
            // either -- its handler is empty beyond a commented-out log
            // line, so nothing to port here.
            1 => None,
            // DL_PLUS_COMMAND
            2 => self.process_dl_plus_command(&data, pos, seg_type),
            _ => None,
        }
    }

    fn process_dl_plus_command(
        &mut self,
        data: &[u8],
        mut pos: usize,
        seg_type: SegmentType,
    ) -> Option<DlsUpdate> {
        self.is_dynamic_plus = true;

        // Field 2 (link bit, + segment number for Intermediate/Last) is
        // parsed by the reference but never used beyond a log line for
        // Intermediate/Last -- and its `linkBit` local is never used at
        // all beyond being computed -- so nothing here actually needs
        // porting past skipping past Field 2's own byte.
        let _ = seg_type;

        // Field 3: DL command length (bytes in the DL Command field minus
        // 1) -- also parsed but not otherwise consumed by the reference.
        pos += 1;

        let &c_id = data.get(pos)?;
        let c_id = (c_id & 0xF0) >> 4;

        if c_id != 0x00 || self.dls_full_data.is_empty() {
            return None;
        }

        let &flags = data.get(pos)?;
        let item_toggle = (flags & 0x08) != 0;
        let item_running = (flags & 0x04) != 0;
        let num_tags = (flags & 0x03) + 1;
        pos += 1;

        let mut plus_tags = Vec::with_capacity(num_tags as usize);
        for _ in 0..num_tags {
            let &content_type_byte = data.get(pos)?;
            let content_type = DlPlusContentType::from_code(content_type_byte & 0x7F);
            pos += 1;
            let &start_marker = data.get(pos)?;
            let start_marker = (start_marker & 0x7F) as usize;
            pos += 1;
            let &length_marker = data.get(pos)?;
            let length_marker = (length_marker & 0x7F) as usize;
            pos += 1;

            let text = if content_type != DlPlusContentType::Dummy {
                let end = (start_marker + length_marker + 1).min(self.dls_full_data.len());
                if start_marker < end {
                    decode_label_text(&self.dls_full_data[start_marker..end], self.current_charset)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            plus_tags.push(DlPlusTag { content_type, text });
        }

        Some(DlsUpdate {
            text: decode_label_text(&self.dls_full_data, self.current_charset),
            charset: self.current_charset,
            item_toggle,
            item_running,
            plus_tags,
        })
    }
}

/// `REMOVE_PADDING`: strips trailing zero bytes.
fn remove_trailing_zero_padding(data: &mut Vec<u8>) {
    while data.last() == Some(&0) {
        data.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc_ccitt_of(data: &[u8]) -> u16 {
        let mut crc: u16 = 0xffff;
        for &byte in data {
            crc = (crc << 8) ^ crate::tables::CRC_CCITT_TABLE[usize::from((crc >> 8) as u8 ^ byte)];
        }
        crc ^ 0xffff
    }

    fn with_crc(mut data: Vec<u8>) -> Vec<u8> {
        let crc = crc_ccitt_of(&data);
        data.push((crc >> 8) as u8);
        data.push(crc as u8);
        data
    }

    #[test]
    fn dl_plus_tag_references_earlier_label_substring() {
        let mut decoder = DlsDecoder::new();

        // Plain ONE_AND_ONLY segment: "HELLO WORLD".
        let text = b"HELLO WORLD";
        let length_field = (text.len() - 1) as u8;
        let byte0 = (0b11 << 5) | length_field; // segType=ONE_AND_ONLY, commandFlag=0
        let byte1 = 0x00; // charset=EBU Latin, rfa=0
        let mut plain = vec![byte0, byte1];
        plain.extend_from_slice(text);
        let plain = with_crc(plain);

        // DL_PLUS_COMMAND segment: 1 tag, ItemArtist (code 4) over
        // "HELLO" (start=0, length_marker=4 -> 5 chars), itemToggle=1.
        // segType=ONE_AND_ONLY (bits 6-5), commandFlag=1 (bit 4), command
        // value=DL_PLUS_COMMAND=2 (bits 3-0) -- all in this one byte.
        let byte0 = (0b11 << 5) | 0x10 | 0x02;
        let byte1 = 0x00; // Field2+Field3, unused downstream
        let byte2 = 0x08; // cId=0, itemToggle=1, itemRunning=0, numTags=1
        let content_type = 4u8; // ItemArtist
        let start_marker = 0u8;
        let length_marker = 4u8; // "HELLO" = 5 chars
        let dl_plus = with_crc(vec![
            byte0,
            byte1,
            byte2,
            content_type,
            start_marker,
            length_marker,
        ]);

        // Each DLS_DATAGROUP_START only *processes* what was stored by an
        // *earlier* start (see `application_data_input`'s doc comment), so
        // this takes 3 calls: store "HELLO WORLD", flush+store DL+, then
        // flush DL+ (which needs `dls_full_data` from the first already
        // populated by then).
        assert!(decoder.application_data_input(&plain, 2).is_none());
        let mid = decoder.application_data_input(&dl_plus, 2);
        assert_eq!(mid.unwrap().text, "HELLO WORLD"); // plain segment, processed here
        let update = decoder
            .application_data_input(&[], 2)
            .expect("DL+ update expected");

        assert_eq!(update.text, "HELLO WORLD");
        assert!(update.item_toggle);
        assert!(!update.item_running);
        assert_eq!(update.plus_tags.len(), 1);
        assert_eq!(
            update.plus_tags[0].content_type,
            DlPlusContentType::ItemArtist
        );
        assert_eq!(update.plus_tags[0].text, "HELLO");
    }
}
