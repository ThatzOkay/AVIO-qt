//! Android Open Accessory Protocol (AOAP) constants.
//! Spec: <https://source.android.com/docs/core/interaction/accessories/aoa>

pub const GOOGLE_VID: u16 = 0x18d1;

// Accessory-mode product IDs (phone re-enumerates after startAccessoryMode).
pub const ACCESSORY_PIDS: [u16; 6] = [
    0x2d00, // Accessory
    0x2d01, // Accessory + ADB
    0x2d02, // Audio
    0x2d03, // Audio + ADB
    0x2d04, // Accessory + Audio
    0x2d05, // Accessory + Audio + ADB
];

// Vendor-specific USB control transfer requests.
pub const REQ_GET_PROTOCOL: u8 = 51;
pub const REQ_SEND_STRING: u8 = 52;
pub const REQ_START: u8 = 53;

// String indices for REQ_SEND_STRING.
pub const STRING_MANUFACTURER: u16 = 0;
pub const STRING_MODEL: u16 = 1;
pub const STRING_DESCRIPTION: u16 = 2;
pub const STRING_VERSION: u16 = 3;
pub const STRING_URI: u16 = 4;
pub const STRING_SERIAL: u16 = 5;

// AOAP-host identification advertised to the phone via SEND_STRING.
//
// IMPORTANT: `MANUFACTURER` and `MODEL` are NOT cosmetic — Android matches them
// against its accessory filters to decide whether to launch Android Auto.
//
// DESCRIPTION / VERSION / URI / SERIAL are free-form and only surface in the
// connection dialog.
pub const AOAP_MANUFACTURER: &str = "Android";
pub const AOAP_MODEL: &str = "Android Auto";
pub const AOAP_DESCRIPTION: &str = "avio Wired Android Auto host";
pub const AOAP_VERSION: &str = "1.0.0";
pub const AOAP_URI: &str = "";
pub const AOAP_SERIAL: &str = "avio-0001";

// Loopback address the bridge advertises for the AA TcpServer to connect to.
pub const AOAP_LOOPBACK_HOST: &str = "127.0.0.1";
pub const AOAP_LOOPBACK_PORT: u16 = 5278;

// AOAP handshake timing. Increase if real phones need more time on slower buses.
pub const AOAP_RE_ENUMERATE_TIMEOUT_MS: u64 = 5_000;
