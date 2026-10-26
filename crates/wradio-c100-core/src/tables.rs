//! Vendor tuning tables for the DAB Band III receiver chip.
//!
//! There's no datasheet for this chip -- these register-value tables and
//! the PLL/ADC-clock arithmetic that indexes them are the only spec that
//! exists for it. Do not reorder, dedupe, or "clean up" any of these
//! tables -- they are opaque vendor protocol / tuning data, transcribed
//! byte-for-byte and verified against the source, not something to
//! reinterpret.

/// ADC clock synthesizer register values, indexed by [`AdcClkFreq`], for a
/// contiguous run of seven clock-synth registers.
pub const ADC_CLK_SYN_TBL: [[u8; 7]; 4] = [
    [0x08, 0x01, 0x13, 0x25, 0x06, 0x7D, 0xB4], // Based 24.576MHz, 8MHz       External Clock21
    [0x06, 0x01, 0x0F, 0x27, 0x07, 0x60, 0xB8], // Based 24.576MHz, 8.192MHz   External Clock20
    [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], // Based 24.576MHz, 9MHz       Unsupported Clock
    [0x0C, 0x01, 0x1F, 0x27, 0x06, 0xE1, 0xB8], // Based 24.576MHz, 9.6MHz     External Clock22
];

/// The ADC sample clock frequencies the tuner can be configured for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdcClkFreq {
    Freq8MHz = 0,
    Freq8192MHz = 1,
    Freq9MHz = 2,
    Freq96MHz = 3,
}

impl AdcClkFreq {
    pub const fn table_row(self) -> &'static [u8; 7] {
        &ADC_CLK_SYN_TBL[self as usize]
    }
}

/// Which ADC clock frequency each Band III channel requires, indexed the
/// same way as [`DAB_FREQ_TABLE_KHZ`] (with the same N-frequency insert
/// offsets applied when resolving a frequency to a table index).
pub const ADC_CLK_TYPE_TBL_DAB_B3: [AdcClkFreq; 41] = {
    use AdcClkFreq::*;
    [
        Freq8192MHz, // 5A : 174928
        Freq96MHz,   // 5B : 176640
        Freq8192MHz, // 5C : 178352
        Freq96MHz,   // 5D : 180064
        Freq8192MHz, // 6A : 181936
        Freq96MHz,   // 6B : 183648
        Freq8192MHz, // 6C : 185360
        Freq8192MHz, // 6D : 187072
        Freq8MHz,    // 7A : 188928
        Freq8192MHz, // 7B : 190640
        Freq8192MHz, // 7C : 192352
        Freq8192MHz, // 7D : 194064
        Freq8MHz,    // 8A : 195936
        Freq8192MHz, // 8B : 197648
        Freq8192MHz, // 8C : 199360
        Freq8MHz,    // 8D : 201072
        Freq8192MHz, // 9A : 202928
        Freq8MHz,    // 9B : 204640
        Freq8192MHz, // 9C : 206352
        Freq96MHz,   // 9D : 208064
        Freq8192MHz, // 10A: 209936
        Freq8192MHz, // 10N: 210096
        Freq8192MHz, // 10B: 211648
        Freq8MHz,    // 10C: 213360
        Freq8192MHz, // 10D: 215072
        Freq96MHz,   // 11A: 216928
        Freq8192MHz, // 11N: 217088
        Freq8192MHz, // 11B: 218640
        Freq8192MHz, // 11C: 220352
        Freq8MHz,    // 11D: 222064
        Freq8192MHz, // 12A: 223936
        Freq8MHz,    // 12N: 224096
        Freq8MHz,    // 12B: 225648
        Freq8192MHz, // 12C: 227360
        Freq8MHz,    // 12D: 229072
        Freq8192MHz, // 13A: 230784
        Freq8192MHz, // 13B: 232496
        Freq8MHz,    // 13C: 234208
        Freq8192MHz, // 13D: 235776
        Freq8MHz,    // 13E: 237488
        Freq8192MHz, // 13F: 239200
    ]
};

/// The lowest Band III channel's frequency, in the same unit as
/// [`DAB_FREQ_TABLE_KHZ`]'s entries (which, despite that name, are kHz
/// values -- 174928 kHz = 174.928 MHz). The frequency-to-table-index
/// arithmetic in `hal::set_frequency` takes its raw input (which is actually
/// in Hz, despite its own naming) and divides by 1000 before diffing it
/// against this constant, so the two divisions/namings cancel out; this is
/// preserved exactly as the vendor arithmetic requires rather than
/// "simplified", since changing it risks silently shifting which table
/// index a given frequency resolves to.
pub const DAB_CH_BAND3_START_FREQ_KHZ: i32 = 174928;
/// Channel spacing within Band III, in the same unit as
/// [`DAB_CH_BAND3_START_FREQ_KHZ`].
pub const DAB_CH_BAND3_STEP_FREQ_KHZ: i32 = 1712;

/// PLL N/F divider values for each Band III channel, written into the PLL
/// configuration registers when tuning.
pub const PLL_NF_DAB_BAND3: [i32; 41] = [
    0x38F3D55, // 5A : 174928
    0x3982800, // 5B : 176640
    0x3A112AA, // 5C : 178352
    0x3A9FD55, // 5D : 180064
    0x3B3BD55, // 6A : 181936
    0x3BCA800, // 6B : 183648
    0x3C592AA, // 6C : 185360
    0x3CE7D55, // 6D : 187072
    0x3D82800, // 7A : 188928
    0x3E112AA, // 7B : 190640
    0x3E9FD55, // 7C : 192352
    0x3F2E800, // 7D : 194064
    0x3FCA800, // 8A : 195936
    0x40592AA, // 8B : 197648
    0x40E7D55, // 8C : 199360
    0x4176800, // 8D : 201072
    0x42112AA, // 9A : 202928
    0x429FD55, // 9B : 204640
    0x432E800, // 9C : 206352
    0x43BD2AA, // 9D : 208064
    0x44592AA, // 10A: 209936
    0x4466800, // 10N: 210096
    0x44E7D55, // 10B: 211648
    0x4576800, // 10C: 213360
    0x46052AA, // 10D: 215072
    0x469FD55, // 11A: 216928
    0x46AD2AA, // 11N: 217088
    0x472E800, // 11B: 218640
    0x47BD2AA, // 11C: 220352
    0x484BD55, // 11D: 222064
    0x48E7D55, // 12A: 223936
    0x48F52AA, // 12N: 224096
    0x4976800, // 12B: 225648
    0x4A052AA, // 12C: 227360
    0x4A93D55, // 12D: 229072
    0x4B22800, // 13A: 230784
    0x4BB12AA, // 13B: 232496
    0x4C3FD55, // 13C: 234208
    0x4CC2800, // 13D: 235776
    0x4D512AA, // 13E: 237488
    0x4DDFD55, // 13F: 239200
];

/// Number of buckets in the antenna/signal-level table.
pub const DAB_MAX_NUM_ANTENNA_LEVEL: usize = 7;

/// CER-period-scaled thresholds bucketing reception quality into a 0..=6
/// "signal bars" level.
pub const ANT_LVL_TBL: [u32; DAB_MAX_NUM_ANTENNA_LEVEL] = [810, 700, 490, 400, 250, 180, 0];

/// Number of Band III channels in the tuning table.
pub const NUM_DAB_ENSEMBLES: usize = 41;

/// All 41 Band III channel frequencies in kHz (e.g. 174928 == 174.928 MHz),
/// including the 3 "intermediate/N" frequencies at indices 21, 26, 31 that a
/// scan skips over (see [`DAB_N_FREQUENCIES_IDX`]).
pub const DAB_FREQ_TABLE_KHZ: [u32; NUM_DAB_ENSEMBLES] = [
    174928, // 05A
    176640, // 05B
    178352, // 05C
    180064, // 05D
    181936, // 06A
    183648, // 06B
    185360, // 06C
    187072, // 06D
    188928, // 07A
    190640, // 07B
    192352, // 07C
    194064, // 07D
    195936, // 08A
    197648, // 08B
    199360, // 08C
    201072, // 08D
    202928, // 09A
    204640, // 09B
    206352, // 09C
    208064, // 09D
    209936, // 10A
    210096, // 10N (intermediate)
    211648, // 10B
    213360, // 10C
    215072, // 10D
    216928, // 11A
    217088, // 11N (intermediate)
    218640, // 11B
    220352, // 11C
    222064, // 11D
    223936, // 12A
    224096, // 12N (intermediate)
    225648, // 12B
    227360, // 12C
    229072, // 12D
    230784, // 13A
    232496, // 13B
    234208, // 13C
    235776, // 13D
    237488, // 13E
    239200, // 13F
];

/// Number of "intermediate/N" channels to skip while scanning.
pub const NUM_DAB_N_FREQUENCIES: usize = 3;

/// Indices into [`DAB_FREQ_TABLE_KHZ`] of the 3 "intermediate" channels a
/// scan skips over.
pub const DAB_N_FREQUENCIES_IDX: [usize; NUM_DAB_N_FREQUENCIES] = [21, 26, 31];

/// This chip can only demodulate one MSC subchannel at a time, which is why
/// an announcement (FIG 0/18/0/19) genuinely interrupts the primary
/// programme rather than ducking under it.
pub const MAXIMUM_CONCURRENT_SUBCHANNELS: u8 = 1;

/// CRC-CCITT (X.25 / DAB PAD) lookup table, used to verify DLS/X-PAD segment
/// CRCs.
pub const CRC_CCITT_TABLE: [u16; 256] = [
    0x0000, 0x1021, 0x2042, 0x3063, 0x4084, 0x50A5, 0x60C6, 0x70E7, 0x8108, 0x9129, 0xA14A, 0xB16B,
    0xC18C, 0xD1AD, 0xE1CE, 0xF1EF, 0x1231, 0x0210, 0x3273, 0x2252, 0x52B5, 0x4294, 0x72F7, 0x62D6,
    0x9339, 0x8318, 0xB37B, 0xA35A, 0xD3BD, 0xC39C, 0xF3FF, 0xE3DE, 0x2462, 0x3443, 0x0420, 0x1401,
    0x64E6, 0x74C7, 0x44A4, 0x5485, 0xA56A, 0xB54B, 0x8528, 0x9509, 0xE5EE, 0xF5CF, 0xC5AC, 0xD58D,
    0x3653, 0x2672, 0x1611, 0x0630, 0x76D7, 0x66F6, 0x5695, 0x46B4, 0xB75B, 0xA77A, 0x9719, 0x8738,
    0xF7DF, 0xE7FE, 0xD79D, 0xC7BC, 0x48C4, 0x58E5, 0x6886, 0x78A7, 0x0840, 0x1861, 0x2802, 0x3823,
    0xC9CC, 0xD9ED, 0xE98E, 0xF9AF, 0x8948, 0x9969, 0xA90A, 0xB92B, 0x5AF5, 0x4AD4, 0x7AB7, 0x6A96,
    0x1A71, 0x0A50, 0x3A33, 0x2A12, 0xDBFD, 0xCBDC, 0xFBBF, 0xEB9E, 0x9B79, 0x8B58, 0xBB3B, 0xAB1A,
    0x6CA6, 0x7C87, 0x4CE4, 0x5CC5, 0x2C22, 0x3C03, 0x0C60, 0x1C41, 0xEDAE, 0xFD8F, 0xCDEC, 0xDDCD,
    0xAD2A, 0xBD0B, 0x8D68, 0x9D49, 0x7E97, 0x6EB6, 0x5ED5, 0x4EF4, 0x3E13, 0x2E32, 0x1E51, 0x0E70,
    0xFF9F, 0xEFBE, 0xDFDD, 0xCFFC, 0xBF1B, 0xAF3A, 0x9F59, 0x8F78, 0x9188, 0x81A9, 0xB1CA, 0xA1EB,
    0xD10C, 0xC12D, 0xF14E, 0xE16F, 0x1080, 0x00A1, 0x30C2, 0x20E3, 0x5004, 0x4025, 0x7046, 0x6067,
    0x83B9, 0x9398, 0xA3FB, 0xB3DA, 0xC33D, 0xD31C, 0xE37F, 0xF35E, 0x02B1, 0x1290, 0x22F3, 0x32D2,
    0x4235, 0x5214, 0x6277, 0x7256, 0xB5EA, 0xA5CB, 0x95A8, 0x8589, 0xF56E, 0xE54F, 0xD52C, 0xC50D,
    0x34E2, 0x24C3, 0x14A0, 0x0481, 0x7466, 0x6447, 0x5424, 0x4405, 0xA7DB, 0xB7FA, 0x8799, 0x97B8,
    0xE75F, 0xF77E, 0xC71D, 0xD73C, 0x26D3, 0x36F2, 0x0691, 0x16B0, 0x6657, 0x7676, 0x4615, 0x5634,
    0xD94C, 0xC96D, 0xF90E, 0xE92F, 0x99C8, 0x89E9, 0xB98A, 0xA9AB, 0x5844, 0x4865, 0x7806, 0x6827,
    0x18C0, 0x08E1, 0x3882, 0x28A3, 0xCB7D, 0xDB5C, 0xEB3F, 0xFB1E, 0x8BF9, 0x9BD8, 0xABBB, 0xBB9A,
    0x4A75, 0x5A54, 0x6A37, 0x7A16, 0x0AF1, 0x1AD0, 0x2AB3, 0x3A92, 0xFD2E, 0xED0F, 0xDD6C, 0xCD4D,
    0xBDAA, 0xAD8B, 0x9DE8, 0x8DC9, 0x7C26, 0x6C07, 0x5C64, 0x4C45, 0x3CA2, 0x2C83, 0x1CE0, 0x0CC1,
    0xEF1F, 0xFF3E, 0xCF5D, 0xDF7C, 0xAF9B, 0xBFBA, 0x8FD9, 0x9FF8, 0x6E17, 0x7E36, 0x4E55, 0x5E74,
    0x2E93, 0x3EB2, 0x0ED1, 0x1EF0,
];

/// Verifies the trailing 2-byte CRC-CCITT of `data` (initial register
/// 0xFFFF, MSB-first transmitted CRC). Returns `false` for inputs shorter
/// than 2 bytes.
pub fn crc_ccitt_check(data: &[u8]) -> bool {
    if data.len() < 2 {
        return false;
    }

    let mut crc: u16 = 0xFFFF;
    let payload_len = data.len() - 2;
    for &byte in &data[..payload_len] {
        crc = (crc << 8) ^ CRC_CCITT_TABLE[usize::from((crc >> 8) as u8 ^ byte)];
    }

    let crc_val = (u16::from(data[payload_len]) << 8) | u16::from(data[payload_len + 1]);
    let crc2 = crc_val ^ 0xFFFF;

    crc == crc2
}

/// EBU Latin character set table used by DAB text fields (ensemble/service
/// labels, DLS). Indexed `[hi nibble][lo nibble]` of the raw byte, each
/// entry a single Unicode codepoint.
pub const EBU_SET: [[char; 16]; 16] = [
    [
        '\u{0000}', '\u{0118}', '\u{012E}', '\u{0172}', '\u{0102}', '\u{0116}', '\u{010E}',
        '\u{0218}', '\u{021A}', '\u{010A}', '\u{0000}', '\u{0000}', '\u{0120}', '\u{0139}',
        '\u{017B}', '\u{0143}',
    ],
    [
        '\u{0105}', '\u{0119}', '\u{012F}', '\u{0173}', '\u{0103}', '\u{0117}', '\u{010F}',
        '\u{0219}', '\u{021B}', '\u{010B}', '\u{0147}', '\u{011A}', '\u{0121}', '\u{013A}',
        '\u{017C}', '\u{0000}',
    ],
    [
        '\u{0020}', '\u{0021}', '\u{0022}', '\u{0023}', '\u{0142}', '\u{0025}', '\u{0026}',
        '\u{0027}', '\u{0028}', '\u{0029}', '\u{002A}', '\u{002B}', '\u{002C}', '\u{002D}',
        '\u{002E}', '\u{002F}',
    ],
    [
        '\u{0030}', '\u{0031}', '\u{0032}', '\u{0033}', '\u{0034}', '\u{0035}', '\u{0036}',
        '\u{0037}', '\u{0038}', '\u{0039}', '\u{003A}', '\u{003B}', '\u{003C}', '\u{003D}',
        '\u{003E}', '\u{003F}',
    ],
    [
        '\u{0040}', '\u{0041}', '\u{0042}', '\u{0043}', '\u{0044}', '\u{0045}', '\u{0046}',
        '\u{0047}', '\u{0048}', '\u{0049}', '\u{004A}', '\u{004B}', '\u{004C}', '\u{004D}',
        '\u{004E}', '\u{004F}',
    ],
    [
        '\u{0050}', '\u{0051}', '\u{0052}', '\u{0053}', '\u{0054}', '\u{0055}', '\u{0056}',
        '\u{0057}', '\u{0058}', '\u{0059}', '\u{005A}', '\u{005B}', '\u{016E}', '\u{005D}',
        '\u{0141}', '\u{005F}',
    ],
    [
        '\u{0104}', '\u{0061}', '\u{0062}', '\u{0063}', '\u{0064}', '\u{0065}', '\u{0066}',
        '\u{0067}', '\u{0068}', '\u{0069}', '\u{006A}', '\u{006B}', '\u{006C}', '\u{006D}',
        '\u{006E}', '\u{006F}',
    ],
    [
        '\u{0070}', '\u{0071}', '\u{0072}', '\u{0073}', '\u{0074}', '\u{0075}', '\u{0076}',
        '\u{0077}', '\u{0078}', '\u{0079}', '\u{007A}', '\u{00AB}', '\u{016F}', '\u{00BB}',
        '\u{013D}', '\u{0126}',
    ],
    [
        '\u{00E1}', '\u{00E0}', '\u{00E9}', '\u{00E8}', '\u{00ED}', '\u{00EC}', '\u{00F3}',
        '\u{00F2}', '\u{00FA}', '\u{00F9}', '\u{00D1}', '\u{00C7}', '\u{015E}', '\u{00DF}',
        '\u{00A1}', '\u{0178}',
    ],
    [
        '\u{00E2}', '\u{00E4}', '\u{00EA}', '\u{00EB}', '\u{00EE}', '\u{00EF}', '\u{00F4}',
        '\u{00F6}', '\u{00FB}', '\u{00FC}', '\u{00F1}', '\u{00E7}', '\u{015F}', '\u{011F}',
        '\u{0131}', '\u{00FF}',
    ],
    [
        '\u{0136}', '\u{0145}', '\u{00A9}', '\u{0122}', '\u{011E}', '\u{011B}', '\u{0148}',
        '\u{0151}', '\u{0150}', '\u{20AC}', '\u{00A3}', '\u{0024}', '\u{0100}', '\u{0112}',
        '\u{012A}', '\u{016A}',
    ],
    [
        '\u{0137}', '\u{0146}', '\u{013B}', '\u{0123}', '\u{013C}', '\u{0130}', '\u{0144}',
        '\u{0171}', '\u{0170}', '\u{00BF}', '\u{013E}', '\u{00B0}', '\u{0101}', '\u{0113}',
        '\u{012B}', '\u{016B}',
    ],
    [
        '\u{00C1}', '\u{00C0}', '\u{00C9}', '\u{00C8}', '\u{00CD}', '\u{00CC}', '\u{00D3}',
        '\u{00D2}', '\u{00DA}', '\u{00D9}', '\u{0158}', '\u{010C}', '\u{0160}', '\u{017D}',
        '\u{00D0}', '\u{013F}',
    ],
    [
        '\u{00C2}', '\u{00C4}', '\u{00CA}', '\u{00CB}', '\u{00CE}', '\u{00CF}', '\u{00D4}',
        '\u{00D6}', '\u{00DB}', '\u{00DC}', '\u{0159}', '\u{010D}', '\u{0161}', '\u{017E}',
        '\u{0111}', '\u{0140}',
    ],
    [
        '\u{00C3}', '\u{00C5}', '\u{00C6}', '\u{0152}', '\u{0177}', '\u{00DD}', '\u{00D5}',
        '\u{00D8}', '\u{00DE}', '\u{014A}', '\u{0154}', '\u{0106}', '\u{015A}', '\u{0179}',
        '\u{0164}', '\u{00F0}',
    ],
    [
        '\u{00E3}', '\u{00E5}', '\u{00E6}', '\u{0153}', '\u{0175}', '\u{00FD}', '\u{00F5}',
        '\u{00F8}', '\u{00FE}', '\u{014B}', '\u{0155}', '\u{0107}', '\u{015B}', '\u{017A}',
        '\u{0165}', '\u{0127}',
    ],
];

/// Decodes an EBU Latin byte string into a Rust `String` via [`EBU_SET`].
pub fn convert_ebu_to_utf(ebu: &[u8]) -> String {
    ebu.iter()
        .map(|&b| EBU_SET[usize::from((b >> 4) & 0x0F)][usize::from(b & 0x0F)])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pll_and_freq_tables_same_length() {
        assert_eq!(PLL_NF_DAB_BAND3.len(), NUM_DAB_ENSEMBLES);
        assert_eq!(DAB_FREQ_TABLE_KHZ.len(), NUM_DAB_ENSEMBLES);
        assert_eq!(ADC_CLK_TYPE_TBL_DAB_B3.len(), NUM_DAB_ENSEMBLES);
    }

    #[test]
    fn crc_ccitt_check_rejects_short_input() {
        assert!(!crc_ccitt_check(&[0x00]));
        assert!(!crc_ccitt_check(&[]));
    }
}
