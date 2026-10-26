/// DAB Band III channels as named by welle.io (`Channels::Channels()` in `various/channels.cpp`):
/// no leading zero on the block number ("5A", not "05A"), and no intermediate "N" channels.
const BAND_III: [(&str, u32); 38] = [
    ("5A", 174_928),
    ("5B", 176_640),
    ("5C", 178_352),
    ("5D", 180_064),
    ("6A", 181_936),
    ("6B", 183_648),
    ("6C", 185_360),
    ("6D", 187_072),
    ("7A", 188_928),
    ("7B", 190_640),
    ("7C", 192_352),
    ("7D", 194_064),
    ("8A", 195_936),
    ("8B", 197_648),
    ("8C", 199_360),
    ("8D", 201_072),
    ("9A", 202_928),
    ("9B", 204_640),
    ("9C", 206_352),
    ("9D", 208_064),
    ("10A", 209_936),
    ("10B", 211_648),
    ("10C", 213_360),
    ("10D", 215_072),
    ("11A", 216_928),
    ("11B", 218_640),
    ("11C", 220_352),
    ("11D", 222_064),
    ("12A", 223_936),
    ("12B", 225_648),
    ("12C", 227_360),
    ("12D", 229_072),
    ("13A", 230_784),
    ("13B", 232_496),
    ("13C", 234_208),
    ("13D", 235_776),
    ("13E", 237_488),
    ("13F", 239_200),
];

/// The welle.io channel name for a frequency in kHz, e.g. `225_648` -> `"12B"`.
pub fn channel_name(frequency_khz: u32) -> Option<&'static str> {
    BAND_III
        .iter()
        .find(|&&(_, khz)| khz == frequency_khz)
        .map(|&(name, _)| name)
}
