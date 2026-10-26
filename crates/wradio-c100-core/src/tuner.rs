//! High-level tuner handle.
//!
//! This is intentionally minimal for now: it wires the USB transport and
//! register HAL together and exposes `initialize`/`tune_khz`, the two
//! signal-quality reads, and a `poll_fic` primitive for pulling raw FIC
//! bytes, enough to verify hardware bring-up and FIC reception end-to-end.
//! It deliberately does *not* own an [`crate::ensemble::Ensemble`] or run
//! its own polling loop yet -- see `examples/scan_and_listen.rs` for how a
//! caller drives `poll_fic` into `fic::parse_fic`/`Ensemble::apply` itself
//! in the meantime. The full async architecture described in the port plan
//! (a background device task owning that loop, a command channel,
//! broadcast/watch channels for ensemble/lock/DLS updates, scan
//! orchestration, USB reconnect handling, ...) lands in a later stage once
//! there's a Tauri plugin (or similar multi-consumer caller) that actually
//! needs it -- a plain example driving the same calls in its own loop
//! doesn't.

use std::time::Duration;

use crate::error::{Error, Result};
use crate::hal::Hal;
use crate::tables;
use crate::usb::UsbDevice;

/// Owns the tuner hardware handle.
pub struct Tuner {
    hal: Hal,
    /// Mirrors `RaonTunerInput::m_currentSubchanId`; 0xFF means none open.
    current_subchan_id: u8,
    /// The last frequency successfully commanded via `tune_khz`, restored
    /// automatically by `reconnect`. In-memory only (a power blip, not the
    /// disk-persisted "last played" station from a later phase).
    last_tuned_khz: Option<u32>,
}

impl Tuner {
    /// Opens the first attached wRadio C100 device.
    pub async fn open() -> Result<Self> {
        let usb = UsbDevice::open_first().await?;
        Ok(Self {
            hal: Hal::new(usb),
            current_subchan_id: 0xFF,
            last_tuned_khz: None,
        })
    }

    /// Runs the chip bring-up sequence. Returns whether it succeeded.
    pub async fn initialize(&mut self) -> Result<bool> {
        self.hal.initialize().await
    }

    /// Tunes to a Band III channel given in kHz (e.g. 174928 for channel
    /// 5A). Does not validate that `khz` is one of the 41 known channels
    /// beyond what the underlying frequency-to-table-index resolution
    /// itself rejects.
    pub async fn tune_khz(&mut self, khz: u32) -> Result<()> {
        // The register-level tuning function historically takes this value
        // pre-multiplied by 1000 -- see `hal::Hal::set_frequency`'s doc
        // comment for why.
        self.hal.set_frequency(khz * 1000).await?;
        self.hal.soft_reset().await?;
        self.last_tuned_khz = Some(khz);
        Ok(())
    }

    /// Tunes to `khz` and waits for full channel lock, retrying with a
    /// fresh tune (the reference's own `softReset` + retune sequence, via
    /// [`Tuner::tune_khz`]) if lock isn't achieved within `lock_timeout`,
    /// up to `max_attempts` times total -- closes the reference README's
    /// own "Reset on failed tuning/excessive CRC errors" TODO. Returns
    /// `Ok(true)` once locked, `Ok(false)` if every attempt was exhausted
    /// without achieving lock (not an error: a channel with no broadcast
    /// on it is an expected outcome, e.g. during a scan).
    pub async fn tune_and_wait_for_lock(
        &mut self,
        khz: u32,
        lock_timeout: Duration,
        max_attempts: u32,
    ) -> Result<bool> {
        for attempt in 1..=max_attempts {
            self.tune_khz(khz).await?;

            let deadline = tokio::time::Instant::now() + lock_timeout;
            while tokio::time::Instant::now() < deadline {
                if self.lock_status().await?.fully_locked() {
                    return Ok(true);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            tracing::debug!(
                attempt,
                max_attempts,
                khz,
                "tuning attempt did not achieve lock, retrying"
            );
        }

        Ok(false)
    }

    /// The full Band III channel table this tuner can be commanded to, in
    /// kHz, in scan order (including the 3 intermediate/N channels a real
    /// scan skips -- see [`tables::DAB_N_FREQUENCIES_IDX`]).
    pub fn channel_table_khz() -> &'static [u32] {
        &tables::DAB_FREQ_TABLE_KHZ
    }

    /// Reads the current OFDM/FEC lock status.
    pub async fn lock_status(&mut self) -> Result<LockStatus> {
        let raw = self.hal.get_lock_status().await?;
        Ok(LockStatus {
            ofdm_locked: raw & crate::hal::RTV_DAB_OFDM_LOCK_MASK != 0,
            fec_locked: raw & crate::hal::RTV_DAB_FEC_LOCK_MASK != 0,
        })
    }

    /// Reads the current smoothed antenna/signal level (0..=6), or `None`
    /// if OFDM isn't locked.
    pub async fn antenna_level(&mut self) -> Result<Option<u8>> {
        self.hal.get_antenna_level().await
    }

    /// Reads lock status and antenna level together into one
    /// [`SignalQuality`] snapshot. See that type's own doc comment for
    /// the `muted`/`device_connected` field caveats.
    pub async fn signal_quality(&mut self) -> Result<SignalQuality> {
        let lock = self.lock_status().await?;
        let signal_bars = self.antenna_level().await?;
        Ok(SignalQuality {
            locked: lock.fully_locked(),
            signal_bars,
            muted: false,
            device_connected: true,
        })
    }

    /// Polls for the device to reappear after a disconnect (same VID/PID),
    /// re-opens it, re-runs [`Tuner::initialize`], and restores the
    /// previously tuned frequency and open subchannel (if any) -- closes
    /// the reference README's own "Reconnect/Restart/die on USB
    /// Disconnect" TODO. Call this when any HAL/tuner call returns
    /// [`Error::DeviceDisconnected`], then retry whatever operation failed.
    ///
    /// Returns only once the device is back and fully re-initialized (or
    /// a different error occurs while trying) -- `poll_interval` paces how
    /// often re-opening/re-initializing is retried while the device is
    /// still absent or not yet ready.
    pub async fn reconnect(&mut self, poll_interval: Duration) -> Result<()> {
        let usb = loop {
            match UsbDevice::open_first().await {
                Ok(usb) => break usb,
                Err(Error::DeviceNotFound) => tokio::time::sleep(poll_interval).await,
                Err(e) => return Err(e),
            }
        };
        self.hal = Hal::new(usb);

        while !self.hal.initialize().await? {
            tokio::time::sleep(poll_interval).await;
        }

        if let Some(khz) = self.last_tuned_khz {
            self.tune_khz(khz).await?;
        }
        if self.current_subchan_id != 0xFF {
            self.hal.open_subchannel(self.current_subchan_id).await?;
        }

        Ok(())
    }

    /// Polls the interrupt status register once and, if FIC data is ready,
    /// reads and returns it -- mirrors `RaonTunerInput::readData`'s FIC
    /// branch. Returns an empty `Vec` if no FIC data was ready this poll;
    /// call this repeatedly (e.g. in a loop with a short sleep) the same
    /// way the reference's `commandProcessing`/`readData` loop does. Feed
    /// a non-empty result through [`crate::fic::parse_fic`] and
    /// [`crate::ensemble::Ensemble::apply`].
    pub async fn poll_fic(&mut self) -> Result<Vec<u8>> {
        let status = self.hal.read_interrupt_status().await?;
        if status & crate::hal::FIC_E_INT != 0 {
            self.hal.read_fic_data().await
        } else {
            Ok(Vec::new())
        }
    }

    /// Opens `subchan_id` for MSC1 data delivery
    /// (`RaonTunerInput::openSubChannel`).
    pub async fn open_subchannel(&mut self, subchan_id: u8) -> Result<()> {
        self.hal.open_subchannel(subchan_id).await?;
        self.current_subchan_id = subchan_id;
        Ok(())
    }

    /// Closes the currently open subchannel, if any
    /// (`RaonTunerInput::closeSubchannel`).
    pub async fn close_subchannel(&mut self) -> Result<()> {
        self.hal.close_subchannel(self.current_subchan_id).await?;
        self.current_subchan_id = 0xFF;
        Ok(())
    }

    /// Polls the interrupt status register once and, if MSC1 data is
    /// ready, reads and returns it -- mirrors `RaonTunerInput::readData`'s
    /// MSC1 branch (an overrun/underrun clears the buffer and yields
    /// nothing this poll, matching the reference). Returns an empty `Vec`
    /// if nothing was ready. Requires a subchannel to already be open via
    /// [`Tuner::open_subchannel`] -- feed a non-empty result into a
    /// [`crate::dabplus::DabPlusDecoder`] for that component's audio.
    ///
    /// Does its own interrupt-status register read rather than sharing one
    /// with [`Tuner::poll_fic`] (the reference reads it once per
    /// `readData` iteration and dispatches both FIC and MSC1 branches off
    /// the same snapshot) -- an extra register round trip per poll, judged
    /// worth it for two independent, separately-callable methods at this
    /// stage; revisit if/when this moves into the single combined device
    /// task the port plan's Async/runtime section describes.
    pub async fn poll_msc(&mut self) -> Result<Vec<u8>> {
        let status = self.hal.read_interrupt_status().await?;
        if status & (crate::hal::MSC1_E_OVER_FLOW | crate::hal::MSC1_E_UNDER_FLOW) != 0 {
            self.hal.clear_msc_buffer().await?;
            return Ok(Vec::new());
        }
        if status & crate::hal::MSC1_E_INT != 0 {
            self.hal.read_msc_data().await
        } else {
            Ok(Vec::new())
        }
    }
}

/// OFDM/FEC lock status, mirroring the two bits the hardware reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockStatus {
    pub ofdm_locked: bool,
    pub fec_locked: bool,
}

impl LockStatus {
    pub fn fully_locked(self) -> bool {
        self.ofdm_locked && self.fec_locked
    }
}

/// Combined signal-quality snapshot, matching the `{ locked, signalBars,
/// muted, deviceConnected }` shape from `RUST_PORT_PROMPT.md`'s "Signal
/// quality & robustness" section (meant to extend a `lock-status` event
/// payload rather than a separate one, once there's an event channel to
/// put it on -- a later stage, per this module's own doc comment).
///
/// `muted` is always `false` here: this crate has no native audio
/// playback of its own to mute (see `dabplus.rs`'s own doc comment on why
/// AAC decoding is out of scope) -- a caller that adds a playback layer
/// should derive its own mute decision from `locked`/`signal_bars` and
/// track it alongside this struct, not read it from here. Kept as a field
/// for shape parity with what a Tauri plugin event will eventually carry.
///
/// `device_connected` is always `true` when this is returned at all: a
/// disconnected device surfaces as `Err(Error::DeviceDisconnected)` from
/// this call itself (there's no channel yet to push a `device_connected:
/// false` value out asynchronously) -- callers should treat that error as
/// `device_connected: false` themselves, and call [`Tuner::reconnect`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalQuality {
    pub locked: bool,
    pub signal_bars: Option<u8>,
    pub muted: bool,
    pub device_connected: bool,
}
