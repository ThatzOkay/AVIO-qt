//! Error types for the wradio-c100 core crate.

use thiserror::Error;

/// Top-level error type for everything in this crate.
#[derive(Debug, Error)]
pub enum Error {
    /// No matching USB device was found during enumeration.
    #[error("no wRadio C100 USB device found (VID:PID 16c0:05dc)")]
    DeviceNotFound,

    /// The USB device was lost mid-session (unplugged, power-cycled by the
    /// host head unit, etc). The tuner should transition to a "device lost"
    /// state and poll for the device to reappear rather than treating this
    /// as fatal.
    #[error("USB device disconnected")]
    DeviceDisconnected,

    /// A USB transfer or enumeration operation failed.
    #[error("USB error: {0}")]
    Usb(#[from] nusb::Error),

    /// A bulk transfer failed (short write, stall, cancellation, ...).
    #[error("USB transfer error: {0}")]
    Transfer(#[from] nusb::transfer::TransferError),

    /// A bulk transfer completed with fewer bytes than the register
    /// protocol requires (e.g. the single-byte ack after a register write,
    /// or the multi-byte reply to a register read).
    #[error("short USB read: expected at least {expected} bytes, got {actual}")]
    ShortRead { expected: usize, actual: usize },

    /// The tuner failed to power up within its retry budget.
    #[error("tuner power-up failed (power register never read back the expected value)")]
    PowerUpFailed,

    /// A commanded frequency isn't one of the supported Band III channels.
    #[error("frequency {khz} kHz is not a supported Band III DAB channel")]
    UnsupportedFrequency { khz: u32 },

    /// Reading or writing the persisted station database / presets file
    /// failed.
    #[error("station store I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The persisted station database / presets file couldn't be
    /// (de)serialized.
    #[error("station store (de)serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
