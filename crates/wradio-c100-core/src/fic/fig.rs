//! Common FIG framing: the byte cursor FIG parsers are built on, the FIG
//! type 0 / type 1 common header fields, and the FIG type 1 label format
//! (used by every `FIG 1/x` and `FIG 2/x` label extension).

use crate::registered_tables::CharacterSet;
use crate::tables::convert_ebu_to_utf;

/// FIG 0 extension numbers (the low 5 bits of a FIG type 0's first data
/// byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Fig00Type {
    EnsembleInformation = 0,
    BasicSubchannelOrganization = 1,
    BasicServiceComponentDefinition = 2,
    ServiceComponentPacketMode = 3,
    ServiceComponentStreamCa = 4,
    ServiceComponentLanguage = 5,
    ServiceLinkingInformation = 6,
    ConfigurationInformation = 7,
    ServiceComponentGlobalDefinition = 8,
    CountryLtoInternationalTable = 9,
    DateAndTime = 10,
    UserapplicationInformation = 13,
    FecSubchannelOrganization = 14,
    ProgrammeType = 17,
    AnnouncementSupport = 18,
    AnnouncementSwitching = 19,
    ServiceComponentInformation = 20,
    FrequencyInformation = 21,
    OeServices = 24,
    OeAnnouncementSupport = 25,
    OeAnnouncementSwitching = 26,
}

/// FIG 1 extension numbers (the low 3 bits of a FIG type 1's first data
/// byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Fig01Type {
    EnsembleLabel = 0,
    ProgrammeServiceLabel = 1,
    ServiceComponentLabel = 4,
    DataServiceLabel = 5,
    XpadUserapplicationLabel = 6,
}

/// A read-only forward cursor over one FIG's data bytes.
///
/// Every accessor returns `None` on underrun instead of indexing past the
/// end of the slice: the reference parsers assume well-formed input and
/// have no such bound checks (undefined behaviour in C++ on a truncated
/// FIG), but FIC data arrives over the air and can genuinely be corrupt or
/// cut short, so parsers built on this cursor bail out cleanly instead.
pub struct FigCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> FigCursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn has_remaining(&self) -> bool {
        self.pos < self.data.len()
    }

    pub fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    pub fn u8(&mut self) -> Option<u8> {
        let b = self.data.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }

    pub fn u16(&mut self) -> Option<u16> {
        let hi = self.u8()? as u16;
        let lo = self.u8()? as u16;
        Some((hi << 8) | lo)
    }

    pub fn u24(&mut self) -> Option<u32> {
        let b2 = self.u8()? as u32;
        let b1 = self.u8()? as u32;
        let b0 = self.u8()? as u32;
        Some((b2 << 16) | (b1 << 8) | b0)
    }

    pub fn u32(&mut self) -> Option<u32> {
        let b3 = self.u8()? as u32;
        let b2 = self.u8()? as u32;
        let b1 = self.u8()? as u32;
        let b0 = self.u8()? as u32;
        Some((b3 << 24) | (b2 << 16) | (b1 << 8) | b0)
    }

    pub fn skip(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.data.len());
    }

    /// Takes the next `n` bytes as a slice, or `None` if fewer than `n`
    /// bytes remain.
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }
}

/// The three common flag bits at the top of every FIG type 0 field.
#[derive(Debug, Clone, Copy)]
pub struct Fig00Header {
    /// C/N: whether this describes the current or the next multiplex
    /// configuration.
    pub is_next_configuration: bool,
    /// OE: whether this describes the tuned ensemble or another one.
    pub is_other_ensemble: bool,
    /// P/D: whether service identifiers in this field are 16-bit
    /// (programme) or 32-bit (data).
    pub is_data_service: bool,
}

impl Fig00Header {
    pub fn parse(first_byte: u8) -> Self {
        Self {
            is_next_configuration: (first_byte & 0x80) != 0,
            is_other_ensemble: (first_byte & 0x40) != 0,
            is_data_service: (first_byte & 0x20) != 0,
        }
    }
}

/// The charset/OE flags at the top of every FIG type 1 field.
#[derive(Debug, Clone, Copy)]
pub struct Fig01Header {
    pub charset: CharacterSet,
    pub is_other_ensemble: bool,
}

impl Fig01Header {
    pub fn parse(first_byte: u8) -> Self {
        Self {
            charset: CharacterSet::from_code((first_byte & 0xF0) >> 4),
            is_other_ensemble: (first_byte & 0x08) != 0,
        }
    }
}

/// A decoded FIG type 1 label: the full (up to 16-character) label plus its
/// abbreviated (up to 8-character) short label.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DabLabel {
    pub text: String,
    pub short_text: String,
}

/// Parses one 18-byte FIG type 1 label field (16 bytes of character data
/// followed by a 2-byte abbreviation mask) and decodes it per `charset`.
///
/// The abbreviation mask's bit `k` (counting from the MSB of the first mask
/// byte) selects character `k` of the label for inclusion in the short
/// label, `k` running 0..16 across both mask bytes.
///
/// The reference implementation has two bugs here that are fixed rather
/// than reproduced: it builds the label via `string::resize(16)` followed
/// by `string::insert(begin(), ...)` instead of an assignment, which
/// (because `insert` prepends rather than overwrites) leaves every decoded
/// label 32 bytes long -- the real 16 characters followed by 16 stray NUL
/// bytes; and its short-label character-index arithmetic uses `i*7` for the
/// second mask byte instead of `i*8`, which is off by one and would drop or
/// duplicate a source character. Both are corrected here.
pub fn parse_label(field: &[u8], charset: CharacterSet) -> Option<DabLabel> {
    if field.len() < 18 {
        return None;
    }

    let raw_label = &field[0..16];
    let mask = ((field[16] as u16) << 8) | field[17] as u16;

    let text = decode_label_text(raw_label, charset);

    let mut short_text = String::new();
    for bit in 0..16u32 {
        if (mask >> (15 - bit)) & 1 != 0 {
            if let Some(ch) = text.chars().nth(bit as usize) {
                short_text.push(ch);
            }
        }
    }

    Some(DabLabel { text, short_text })
}

/// Decodes a raw label byte string per its signalled character set.
///
/// The reference implementation defines an EBU-Latin-to-UTF-8 conversion
/// table (`convertEbuToUtf`, see `tables::convert_ebu_to_utf`) but never
/// actually calls it anywhere -- labels are stored as their raw bytes
/// stuffed into a `std::string`. Wired up here so labels using the EBU
/// Latin charset (the common case for DAB service/ensemble labels) decode
/// to correct text instead of raw bytes.
pub(crate) fn decode_label_text(raw: &[u8], charset: CharacterSet) -> String {
    match charset {
        CharacterSet::Utf8 => String::from_utf8_lossy(raw)
            .trim_end_matches('\0')
            .to_string(),
        CharacterSet::Ucs2 => raw
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .take_while(|&code_unit| code_unit != 0)
            .map(|code_unit| char::from_u32(code_unit as u32).unwrap_or('\u{FFFD}'))
            .collect(),
        // EBU Latin, or an unrecognized/unsignalled charset: EBU Latin is
        // the default assumed by DAB receivers when nothing else is
        // signalled, matching the reference's default charset of 0.
        _ => convert_ebu_to_utf(raw).trim_end_matches('\0').to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_label_bit_positions() {
        // "ABCDEFGHIJKLMNOP" with a mask selecting chars 0,7,8,15 (one bit
        // per mask byte at each end) exercises both mask bytes.
        let mut field = b"ABCDEFGHIJKLMNOP".to_vec();
        // bit0 (char0) and bit7 (char7) set in byte0: 1000_0001 = 0x81
        // bit8 (char8) and bit15 (char15) set in byte1: 1000_0001 = 0x81
        field.push(0x81);
        field.push(0x81);
        let label = parse_label(&field, CharacterSet::Utf8).unwrap();
        assert_eq!(label.text, "ABCDEFGHIJKLMNOP");
        assert_eq!(label.short_text, "AHIP");
    }
}
