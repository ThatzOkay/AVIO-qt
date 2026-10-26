//! Tuner chip bring-up and tuning HAL.
//!
//! There is no datasheet for this chip: every register address, bitmask,
//! and table value here is transcribed byte-for-byte from the reference
//! implementation, in the same order, and must stay that way -- treat all
//! of it as opaque vendor protocol, not something to reinterpret.
//!
//! Scope for this module: the USB register read/write/page-switch framing,
//! tuner power-up, ADC clock configuration, the Band III PLL tuning table,
//! frequency tuning, and the FIC/MSC interrupt-status/memory-read register
//! primitives (`read_interrupt_status`/`read_fic_data`/`read_msc_data`/
//! `clear_msc_buffer`). Lock-status and antenna-level reads are also
//! included here since they're small, dependency-free register reads
//! useful for verifying a tune succeeded -- the fuller CER-based
//! signal-bar bucketing / auto-mute / auto-reset behavior that wraps them
//! is built on top of these same functions in a later stage of the port.
//!
//! Not yet ported here: subchannel open/close (`openSubChannel`/
//! `closeSubchannel`) and everything downstream of a raw MSC byte read --
//! superframe sync, Reed-Solomon decode, AU splitting -- those belong to
//! the DAB+ extraction stage. `read_msc_data` exists at the register level
//! already (mirroring the reference's own function grouping) but has
//! nothing meaningful to read until a subchannel is open.

// Several register values below are computed as `CONST & 0x00` or
// `0x00 | value` -- numerically pointless in isolation, but transcribed
// exactly as the reference bit-mask/bit-or expressions they came from
// (a mask of 0x00, an OR with 0x00 acting as a documented "no bits set here"
// placeholder). Left as arithmetic rather than folded to the constant so a
// register sequence here can still be diffed operation-by-operation against
// the source it was transcribed from.
#![allow(clippy::erasing_op, clippy::identity_op)]

use std::time::Duration;

use tracing::warn;

use crate::error::{Error, Result};
use crate::tables::{self, AdcClkFreq};
use crate::usb::UsbDevice;

/// Register page selector: registers are grouped into pages, switched via a
/// dedicated page-select register before addressing anything on that page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RegisterPage {
    Ofdm = 0x02, // For 1seg
    Fec = 0x03,  // For 1seg
    Comm = 0x04,
    Fm = 0x06, // T-DMB OFDM/FM
    Host = 0x07,
    Cas = 0x08,
    Dd = 0x09, // FEC for T-DMB, DAB, FM
    Fic = 0x0A,
    Msc0 = 0x0B,
    Msc1 = 0x0C,
    Rf = 0x0F,
}

/// OFDM lock bit in the combined lock-status value.
pub const RTV_DAB_OFDM_LOCK_MASK: u8 = 0x01;
/// FEC lock bit in the combined lock-status value.
pub const RTV_DAB_FEC_LOCK_MASK: u8 = 0x02;
/// Both lock bits set: full channel lock.
pub const RTV_DAB_CHANNEL_LOCK_OK: u8 = RTV_DAB_OFDM_LOCK_MASK | RTV_DAB_FEC_LOCK_MASK;

// `INT_E_STATL` (0x33, DD page) bit layout, per `raontunerinput.h`:
// [7] OFDM lock [6] MSC1 overrun [5] MSC1 underrun [4] MSC1 int
// [3] MSC0 overrun [2] MSC0 underrun [1] MSC0 int [0] FIC int.
/// FIC data ready.
pub const FIC_E_INT: u8 = 0x01;
/// MSC0 data ready.
pub const MSC0_E_INT: u8 = 0x02;
/// MSC0 underrun.
pub const MSC0_E_UNDER_FLOW: u8 = 0x04;
/// MSC0 overrun.
pub const MSC0_E_OVER_FLOW: u8 = 0x08;
/// MSC1 data ready.
pub const MSC1_E_INT: u8 = 0x10;
/// MSC1 underrun.
pub const MSC1_E_UNDER_FLOW: u8 = 0x20;
/// MSC1 overrun.
pub const MSC1_E_OVER_FLOW: u8 = 0x40;

/// Interrupt status register (DD page) -- see the `INT_E_STATL` bit layout
/// comment above.
const INT_E_STATL: u8 = 0x33;
/// Interrupt clear register (DD page). Its own bit layout, per
/// `raontunerinput.h`, is independent of `INT_E_STATL`'s above: bit 2 MSC1
/// int clear, bit 1 MSC0 int clear, bit 0 FIC int clear.
const INT_E_UCLRL: u8 = 0x35;
const UCLR_FIC_INT: u8 = 0x01;
const UCLR_MSC1_INT: u8 = 0x04;

/// Timeout applied to every register read/write bulk transfer.
const IO_TIMEOUT: Duration = Duration::from_millis(1000);

/// Owns the USB device and the register-protocol framing, plus the small
/// bit of soft state (the smoothed antenna level) carried between reads.
pub struct Hal {
    usb: UsbDevice,
    prev_antenna_lvl: u8,
}

impl Hal {
    pub fn new(usb: UsbDevice) -> Self {
        Self {
            usb,
            prev_antenna_lvl: 0,
        }
    }

    // -- register protocol --

    /// Selects a register page for subsequent reads/writes.
    pub async fn switch_page(&mut self, page: RegisterPage) -> Result<()> {
        self.set_register(0x03, page as u8).await
    }

    /// Writes a value to a register on the currently selected page, then
    /// reads back a single acknowledgement byte (its value isn't checked --
    /// there's no documented error code to check it against).
    pub async fn set_register(&mut self, reg: u8, val: u8) -> Result<()> {
        let set_reg_data = vec![0x21, 0x00, 0x00, 0x02, reg, val];
        self.usb.bulk_write(set_reg_data, IO_TIMEOUT).await?;

        let _ack = self.usb.bulk_read(1, IO_TIMEOUT).await?;
        Ok(())
    }

    /// Reads a register on the currently selected page.
    pub async fn read_register(&mut self, reg: u8) -> Result<u8> {
        let read_reg_req_data = vec![0x22, 0x00, 0x01, 0x00, reg];
        self.usb.bulk_write(read_reg_req_data, IO_TIMEOUT).await?;

        let read_buffer = self.usb.bulk_read(5, IO_TIMEOUT).await?;
        if read_buffer.len() < 5 {
            return Err(Error::ShortRead {
                expected: 5,
                actual: read_buffer.len(),
            });
        }
        Ok(read_buffer[4])
    }

    // -- bring-up --

    /// Runs the full chip bring-up sequence: power-up, clocking, OFDM/FEC
    /// init, interrupt and host-interface configuration, RF init, and
    /// memory/threshold setup. Returns `Ok(true)` once bring-up has
    /// completed; `Ok(false)` if power-up itself failed (nothing else is
    /// attempted in that case).
    pub async fn initialize(&mut self) -> Result<bool> {
        if !self.tuner_power_up().await? {
            return Ok(false);
        }

        self.configure_power_type().await?;
        self.configure_add_clock().await?;
        self.tdmb_init_top().await?;
        self.tdmb_init_comm().await?;

        self.tdmb_init_host().await?;
        self.tdmb_init_ofdm().await?;
        self.tdmb_init_fec().await?;

        self.rtv_reset_memory_fic().await?;
        self.rtv_oem_configure_interrupt().await?;
        self.rtv_stream_disable().await?;
        self.rtv_configure_host_if().await?;
        self.rtv_rf_initialize().await?;
        self.rtv_rf_special().await?;

        self.setup_msc_threshold().await?;
        self.setup_memory_fic().await?;

        Ok(true)
    }

    /// Retries a power-up register write/read-back cycle up to 100 times,
    /// returning as soon as the chip reports itself powered.
    pub async fn tuner_power_up(&mut self) -> Result<bool> {
        for _ in 0..100 {
            self.switch_page(RegisterPage::Host).await?;
            self.set_register(0x7D, 0x06).await?;
            if self.read_register(0x7D).await? == 0x06 {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Configures the chip's I/O and DC/DC power domain settings.
    pub async fn configure_power_type(&mut self) -> Result<()> {
        let reg30 = 0xF2u8 & 0xF0; // IOLDOCON

        let io_type = 0x02u8; // 1.8V I/O

        let reg30 = reg30 | (io_type << 1); // IO Type Select.

        let reg2f = 0x61u8; // DCDC_OUTSEL = 0x03
        let reg52 = 0x07u8; // LDODIG_HT = 0x07
        let reg54 = 0x1Cu8;

        let reg2f = reg2f | 0x10; // PDDCDC_I2C = 1, PDLDO12_I2C = 0

        self.switch_page(RegisterPage::Rf).await?;

        self.set_register(0x54, reg54).await?;
        self.set_register(0x52, reg52).await?;
        self.set_register(0x30, reg30).await?;
        self.set_register(0x2F, reg2f).await?;
        Ok(())
    }

    /// Programs the default (8.192 MHz) ADC clock synthesizer settings and
    /// waits for the synthesizer to report lock.
    pub async fn configure_add_clock(&mut self) -> Result<()> {
        let rege8 = 0x46u8 & 0xC0;
        let rege9 = 0x54u8 & 0xF0;
        let regea = 0x07u8 & 0xC0;
        let regeb = 0x27u8 & 0xC0;
        let regec = 0x1Eu8 & 0xC0;
        let reged = 0x18u8 & 0x00;
        let regee = 0xB8u8 & 0x00;

        self.switch_page(RegisterPage::Rf).await?;

        let row = tables::ADC_CLK_SYN_TBL[0];
        self.set_register(0xE8, rege8 | row[0]).await?;
        self.set_register(0xE9, rege9 | row[1]).await?;
        self.set_register(0xEA, regea | row[2]).await?;
        self.set_register(0xEB, regeb | row[3]).await?;
        self.set_register(0xEC, regec | row[4]).await?;
        self.set_register(0xED, reged | row[5]).await?;
        self.set_register(0xEE, regee | row[6]).await?;

        for _ in 0..10 {
            let rd15 = self.read_register(0x15).await? & 0x01;
            if rd15 > 0 {
                break;
            } else {
                warn!("ADC clock synthesizer: first-step lock not yet achieved, retrying");
            }
        }
        Ok(())
    }

    /// Reprograms the ADC clock synthesizer for a specific clock frequency
    /// (needed per-channel, since different Band III channels require
    /// different ADC sample rates), waits for synthesizer lock, and applies
    /// the matching front-end register set for that clock frequency.
    /// Returns whether synthesizer lock was achieved.
    pub async fn changed_adc_clock(&mut self, adc_clk_type: AdcClkFreq) -> Result<bool> {
        let rege8 = 0x46u8 & 0xC0;
        let rege9 = 0x54u8 & 0xF0;
        let regea = 0x07u8 & 0xC0;
        let regeb = 0x27u8 & 0xC0;
        let regec = 0x1Eu8 & 0xC0;
        let reged = 0x18u8 & 0x00;
        let regee = 0xB8u8 & 0x00;

        self.switch_page(RegisterPage::Rf).await?;

        let row = tables::ADC_CLK_SYN_TBL[adc_clk_type as usize];
        self.set_register(0xE8, rege8 | row[0]).await?;
        self.set_register(0xE9, rege9 | row[1]).await?;
        self.set_register(0xEA, regea | row[2]).await?;
        self.set_register(0xEB, regeb | row[3]).await?;
        self.set_register(0xEC, regec | row[4]).await?;
        self.set_register(0xED, reged | row[5]).await?;
        self.set_register(0xEE, regee | row[6]).await?;

        let mut set_ok = false;
        for _ in 0..10 {
            let rd15 = self.read_register(0x15).await? & 0x01;
            if rd15 > 0 {
                set_ok = true;
                break;
            }
        }

        match adc_clk_type {
            AdcClkFreq::Freq8MHz => {
                self.switch_page(RegisterPage::Comm).await?;
                self.set_register(0x6A, 0x01).await?;

                self.switch_page(RegisterPage::Fm).await?;
                self.set_register(0x3c, 0x4B).await?;
                self.set_register(0x3d, 0x37).await?;
                self.set_register(0x3e, 0x89).await?;
                self.set_register(0x3f, 0x41).await?;
                self.set_register(0x54, 0x58).await?;

                self.set_register(0x40, 0x8F).await?; // PNCO
                self.set_register(0x41, 0xC2).await?; // PNCO
                self.set_register(0x42, 0xF5).await?; // PNCO
                self.set_register(0x43, 0x00).await?; // PNCO
            }
            AdcClkFreq::Freq8192MHz => {
                self.switch_page(RegisterPage::Comm).await?;
                self.set_register(0x6A, 0x01).await?;

                self.switch_page(RegisterPage::Fm).await?;
                self.set_register(0x3c, 0x00).await?;
                self.set_register(0x3d, 0x00).await?;
                self.set_register(0x3e, 0x00).await?;
                self.set_register(0x3f, 0x40).await?;
                self.set_register(0x54, 0x58).await?;

                self.set_register(0x40, 0x00).await?; // PNCO
                self.set_register(0x41, 0x00).await?; // PNCO
                self.set_register(0x42, 0xF0).await?; // PNCO
                self.set_register(0x43, 0x00).await?; // PNCO
            }
            AdcClkFreq::Freq9MHz => {
                self.switch_page(RegisterPage::Comm).await?;
                self.set_register(0x6A, 0x01).await?;

                self.switch_page(RegisterPage::Fm).await?;
                self.set_register(0x3c, 0xB5).await?;
                self.set_register(0x3d, 0x14).await?;
                self.set_register(0x3e, 0x41).await?;
                self.set_register(0x3f, 0x3A).await?;
                self.set_register(0x54, 0x58).await?;

                self.set_register(0x40, 0x0D).await?; // PNCO
                self.set_register(0x41, 0x74).await?; // PNCO
                self.set_register(0x42, 0xDA).await?; // PNCO
                self.set_register(0x43, 0x00).await?; // PNCO
            }
            AdcClkFreq::Freq96MHz => {
                self.switch_page(RegisterPage::Comm).await?;
                self.set_register(0x6A, 0x31).await?;

                self.switch_page(RegisterPage::Fm).await?;
                self.set_register(0x3c, 0x69).await?;
                self.set_register(0x3d, 0x03).await?;
                self.set_register(0x3e, 0x9D).await?;
                self.set_register(0x3f, 0x36).await?;
                self.set_register(0x54, 0x58).await?;

                self.set_register(0x40, 0xCC).await?; // PNCO
                self.set_register(0x41, 0xCC).await?; // PNCO
                self.set_register(0x42, 0xCC).await?; // PNCO
                self.set_register(0x43, 0x00).await?; // PNCO
            }
        }

        Ok(set_ok)
    }

    /// Top-level demodulator init registers.
    pub async fn tdmb_init_top(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Fm).await?;

        self.set_register(0x07, 0x08).await?;
        self.set_register(0x05, 0x17).await?;
        self.set_register(0x06, 0x10).await?;
        self.set_register(0x0A, 0x00).await?;
        Ok(())
    }

    /// Common-block init registers (TS output / error-packet handling, FEC
    /// enable).
    pub async fn tdmb_init_comm(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Comm).await?;

        self.set_register(0x10, 0x91).await?;
        self.set_register(0xE1, 0x00).await?;

        self.set_register(0x35, 0x7B).await?;
        self.set_register(0x3B, 0x3C).await?;

        self.set_register(0x36, 0x68).await?;
        self.set_register(0x3A, 0x44).await?;

        self.set_register(0x3C, 0x20).await?;
        self.set_register(0x3D, 0x0B).await?;
        self.set_register(0x3D, 0x09).await?;

        // 0x30 ==>NO TSOUT@Error packet, 0x10 ==> NULL PID PACKET@Error packet
        self.set_register(0xA6, 0x30).await?;

        // Enable 0x47 insertion to video frame.
        self.set_register(0xAA, 0x01).await?;

        self.set_register(0xAF, 0x07).await?; // FEC
        Ok(())
    }

    /// Host-interface init registers.
    pub async fn tdmb_init_host(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Host).await?;

        self.set_register(0x10, 0x00).await?;
        self.set_register(0x13, 0x16).await?;
        self.set_register(0x14, 0x00).await?;
        self.set_register(0x19, 0x0A).await?;
        self.set_register(0xF0, 0x00).await?;
        self.set_register(0xF1, 0x00).await?;
        self.set_register(0xF2, 0x00).await?;
        self.set_register(0xF3, 0x00).await?;
        self.set_register(0xF4, 0x00).await?;
        self.set_register(0xF5, 0x00).await?;
        self.set_register(0xF6, 0x00).await?;
        self.set_register(0xF7, 0x00).await?;
        self.set_register(0xF8, 0x00).await?;
        self.set_register(0xFB, 0xFF).await?;
        Ok(())
    }

    /// OFDM demodulator init registers (AGC, sync, timing).
    pub async fn tdmb_init_ofdm(&mut self) -> Result<()> {
        let inv_mode = 0x1u8;
        let pwm_com = 0x08u8;
        let wagc_com = 0x03u8;
        let agc_mode = 0x06u8;
        let post_init = 0x09u8;
        let agc_cycle = 0x10u8;

        self.switch_page(RegisterPage::Fm).await?;

        self.set_register(0x12, 0x04).await?;
        self.set_register(0x13, 0x72).await?;
        self.set_register(0x14, 0x63).await?;
        self.set_register(0x15, 0x64).await?;
        self.set_register(0x16, 0x6C).await?;
        self.set_register(0x38, 0x01).await?;

        self.set_register(0x1A, 0xB4).await?;

        self.set_register(0x20, 0x5B).await?;
        self.set_register(0x25, 0x09).await?;

        self.set_register(0x44, 0x00 | post_init).await?;

        self.set_register(0x46, 0xA0).await?;
        self.set_register(0x47, 0x0F).await?;

        self.set_register(0x48, 0xB8).await?;
        self.set_register(0x49, 0x0B).await?;
        self.set_register(0x54, 0x58).await?;

        self.set_register(0x55, 0x06).await?;

        self.set_register(0x56, 0x00 | agc_cycle).await?;

        self.set_register(0x59, 0x51).await?;

        self.set_register(0x5A, 0x1C).await?;

        self.set_register(0x6D, 0x00).await?;
        self.set_register(0x8B, 0x34).await?;
        self.set_register(0x6A, 0x1C).await?;

        self.set_register(0x6B, 0x2D).await?;
        self.set_register(0x85, 0x32).await?;

        self.set_register(0x8D, 0x0C).await?;
        self.set_register(0x8E, 0x01).await?;

        self.set_register(0x33, 0x00 | (inv_mode << 1)).await?;
        self.set_register(0x53, 0x00 | agc_mode).await?;

        self.set_register(0x6F, 0x00 | wagc_com).await?;

        self.set_register(0xBA, pwm_com).await?;

        self.switch_page(RegisterPage::Comm).await?;

        self.set_register(0x6A, 0x01).await?;

        self.switch_page(RegisterPage::Fm).await?; // instead of OFDM page

        self.set_register(0x3C, 0x4B).await?;
        self.set_register(0x3D, 0x37).await?;
        self.set_register(0x3E, 0x89).await?;
        self.set_register(0x3F, 0x41).await?;

        self.set_register(0x40, 0x8F).await?;

        self.set_register(0x41, 0xC2).await?;

        self.set_register(0x42, 0xF5).await?;
        self.set_register(0x43, 0x00).await?;

        self.set_register(0x94, 0x08).await?;

        self.set_register(0x98, 0x05).await?;
        self.set_register(0x99, 0x03).await?;
        self.set_register(0x9B, 0xCF).await?;
        self.set_register(0x9C, 0x10).await?;
        self.set_register(0x9D, 0x1C).await?;
        self.set_register(0x9F, 0x32).await?;
        self.set_register(0xA0, 0x90).await?;

        self.set_register(0xA2, 0xA0).await?;

        self.set_register(0xA3, 0x08).await?;

        self.set_register(0xA4, 0x01).await?;

        self.set_register(0xA8, 0xF6).await?;
        self.set_register(0xA9, 0x89).await?;
        self.set_register(0xAA, 0x0C).await?;
        self.set_register(0xAB, 0x32).await?;

        self.set_register(0xAC, 0x14).await?;
        self.set_register(0xAD, 0x09).await?;

        self.set_register(0xAE, 0xFF).await?;

        self.set_register(0xEB, 0x6B).await?;

        self.set_register(0x93, 0x10).await?;
        self.set_register(0x94, 0x29).await?;
        self.set_register(0xA2, 0x50).await?;
        self.set_register(0xA4, 0x02).await?;
        self.set_register(0xAF, 0x01).await?;
        Ok(())
    }

    /// FEC block init registers.
    pub async fn tdmb_init_fec(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Dd).await?; // instead of FEC_PAGE

        self.set_register(0x80, 0x80).await?;
        self.set_register(0x81, 0xFF).await?;
        self.set_register(0x87, 0x07).await?;

        self.set_register(0x45, 0xA1).await?;

        self.set_register(0xDD, 0xD0).await?;
        self.set_register(0x39, 0x07).await?;
        self.set_register(0xE6, 0x10).await?;
        self.set_register(0xA5, 0xA0).await?;
        Ok(())
    }

    /// Resets the FIC memory before use.
    pub async fn rtv_reset_memory_fic(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Dd).await?;
        self.set_register(0x46, 0x00).await?;
        Ok(())
    }

    /// Configures the two hardware interrupt output pins' mode and polarity.
    pub async fn rtv_oem_configure_interrupt(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Dd).await?;

        // [6]INT1 [5]INT0 - 1: Input mode, 0: Output mode
        self.set_register(0x09, 0x00).await?;

        // [2]INT1 PAD disable [1]INT0 PAD disable
        self.set_register(0x0B, 0x00).await?;

        self.switch_page(RegisterPage::Host).await?;

        // [5:3]INT1 out sel [2:0] INI0 out sel - 0:Toggle 1:Level,, 2:"0", 3:"1"
        self.set_register(0x28, 0x01).await?;

        // [5]INT1 pol [4]INT0 pol - 0:Active High, 1:Active Low [3:0] Period = (INT_TIME+1)/8MHz
        self.set_register(0x2A, 0x13).await?;
        Ok(())
    }

    /// Disables stream output ahead of reconfiguration.
    pub async fn rtv_stream_disable(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Host).await?;
        self.set_register(0x29, 0x08).await?;
        Ok(())
    }

    /// Configures the host interface (SPI mode enable).
    pub async fn rtv_configure_host_if(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Host).await?;

        self.set_register(0x77, 0x14).await?; // SPI Mode Enable
        self.set_register(0x04, 0x28).await?; // SPI Mode Enable
        self.set_register(0x0C, 0xF5).await?;
        Ok(())
    }

    /// Full RF front-end register initialization, plus an auto-detected
    /// variant selection (LNA/front-end trim) based on a register readback
    /// at start-up.
    pub async fn rtv_rf_initialize(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Rf).await?;

        self.set_register(0x27, 0x96).await?;
        self.set_register(0x2B, 0x88).await?;
        self.set_register(0x2D, 0xEC).await?;
        self.set_register(0x2E, 0xB0).await?;
        self.set_register(0x31, 0x04).await?;
        self.set_register(0x34, 0xF8).await?;
        self.set_register(0x35, 0x14).await?;
        self.set_register(0x3A, 0x62).await?;
        self.set_register(0x3B, 0x74).await?;
        self.set_register(0x3C, 0xA8).await?;
        self.set_register(0x43, 0x36).await?;
        self.set_register(0x44, 0x70).await?;
        self.set_register(0x46, 0x07).await?;
        self.set_register(0x49, 0x1F).await?;
        self.set_register(0x4A, 0x50).await?;
        self.set_register(0x4B, 0x80).await?;
        self.set_register(0x53, 0x20).await?;
        self.set_register(0x55, 0xD6).await?;
        self.set_register(0x5A, 0x83).await?;
        self.set_register(0x60, 0x13).await?;
        self.set_register(0x6B, 0x85).await?;
        self.set_register(0x6E, 0x88).await?;
        self.set_register(0x6F, 0x78).await?;
        self.set_register(0x72, 0xF0).await?;
        self.set_register(0x73, 0xCA).await?;
        self.set_register(0x77, 0x80).await?;
        self.set_register(0x78, 0x47).await?;
        self.set_register(0x84, 0x80).await?;
        self.set_register(0x85, 0x90).await?;
        self.set_register(0x86, 0x80).await?;
        self.set_register(0x87, 0x98).await?;
        self.set_register(0x8A, 0xF6).await?;
        self.set_register(0x8B, 0x80).await?;
        self.set_register(0x8C, 0x75).await?;
        self.set_register(0x8F, 0xF9).await?;
        self.set_register(0x90, 0x91).await?;
        self.set_register(0x93, 0x2D).await?;
        self.set_register(0x94, 0x23).await?;
        self.set_register(0x99, 0x77).await?;
        self.set_register(0x9A, 0x2D).await?;
        self.set_register(0x9B, 0x1E).await?;
        self.set_register(0x9C, 0x47).await?;
        self.set_register(0x9D, 0x3A).await?;
        self.set_register(0x9E, 0x03).await?;
        self.set_register(0x9F, 0x1E).await?;
        self.set_register(0xA0, 0x22).await?;
        self.set_register(0xA1, 0x33).await?;
        self.set_register(0xA2, 0x51).await?;
        self.set_register(0xA3, 0x36).await?;
        self.set_register(0xA4, 0x0C).await?;
        self.set_register(0xAE, 0x37).await?;
        self.set_register(0xB5, 0x9B).await?;
        self.set_register(0xBD, 0x45).await?;
        self.set_register(0xBE, 0x68).await?;
        self.set_register(0xBF, 0x5C).await?;
        self.set_register(0xC0, 0x33).await?;
        self.set_register(0xC1, 0xCA).await?;
        self.set_register(0xC2, 0x43).await?;
        self.set_register(0xC3, 0x90).await?;
        self.set_register(0xC4, 0xEC).await?;
        self.set_register(0xC5, 0x17).await?;
        self.set_register(0xC6, 0xDA).await?;
        self.set_register(0xC7, 0x41).await?;
        self.set_register(0xC8, 0x49).await?;
        self.set_register(0xC9, 0xBC).await?;
        self.set_register(0xCA, 0x3A).await?;
        self.set_register(0xCB, 0x20).await?;
        self.set_register(0xCC, 0x43).await?;
        self.set_register(0xCD, 0xA6).await?;
        self.set_register(0xCE, 0x4C).await?;
        self.set_register(0xCF, 0x1F).await?;
        self.set_register(0xD0, 0x8D).await?;
        self.set_register(0xD1, 0x3D).await?;
        self.set_register(0xD2, 0xDF).await?;
        self.set_register(0xD3, 0xA4).await?;
        self.set_register(0xD4, 0x30).await?;
        self.set_register(0xD5, 0x00).await?;
        self.set_register(0xD6, 0x41).await?;
        self.set_register(0xD7, 0x81).await?;
        self.set_register(0xD8, 0xD0).await?;
        self.set_register(0xD9, 0x00).await?;
        self.set_register(0xDA, 0x00).await?;
        self.set_register(0xDB, 0x41).await?;
        self.set_register(0xDC, 0x6D).await?;
        self.set_register(0xDD, 0xAC).await?;
        self.set_register(0xDE, 0x39).await?;
        self.set_register(0xDF, 0x4E).await?;
        self.set_register(0xE0, 0x43).await?;
        self.set_register(0xE1, 0xA0).await?;
        self.set_register(0xE2, 0x4C).await?;
        self.set_register(0xE3, 0x1D).await?;
        self.set_register(0xE4, 0x99).await?;
        self.set_register(0xE5, 0x3B).await?;
        self.set_register(0xE6, 0x00).await?;
        self.set_register(0xE7, 0x0D).await?;
        self.set_register(0xA5, 0x00).await?;
        self.set_register(0xE9, 0x41).await?;
        self.set_register(0xAE, 0x77).await?;
        self.set_register(0xE9, 0x51).await?;

        // Auto LNA from above definition
        self.set_register(0x37, 0x3B).await?;
        self.set_register(0x39, 0x2C).await?;

        let check_val = ((self.read_register(0x10).await? as u16) << 8)
            | self.read_register(0x11).await? as u16;
        if check_val != 0xFFFF {
            self.set_register(0x2C, 0x48).await?;
            self.set_register(0x47, 0xE0).await?;
            self.set_register(0x35, 0x04).await?;

            self.set_register(0x2A, 0x05).await?;
            self.set_register(0x2D, 0x8C).await?;
            self.set_register(0x61, 0x25).await?;
        } else {
            self.set_register(0x2C, 0xC8).await?;
            self.set_register(0x47, 0xB0).await?;
            self.set_register(0x35, 0x04).await?;

            self.set_register(0x2A, 0x07).await?;
            self.set_register(0x2D, 0xEC).await?;
            self.set_register(0x61, 0x2A).await?;
        }

        self.switch_page(RegisterPage::Rf).await?;
        self.set_register(0x6B, 0xC5).await?;
        Ok(())
    }

    /// A short sequence of raw bulk transfers outside the register
    /// read/write framing entirely.
    // TODO(unverified): this sequence's purpose isn't documented anywhere
    // in the reference sources; it's reproduced exactly rather than guessed
    // at.
    pub async fn rtv_rf_special(&mut self) -> Result<()> {
        let read_req = vec![0x10, 0x00, 0x00, 0x01];
        self.usb.bulk_write(read_req, IO_TIMEOUT).await?;
        self.usb.bulk_read(4, IO_TIMEOUT).await?;

        let read_req = vec![0x00, 0x00, 0x00, 0x00];
        self.usb.bulk_write(read_req, IO_TIMEOUT).await?;
        self.usb.bulk_read(4, IO_TIMEOUT).await?;

        let read_req = vec![0x01, 0x00, 0x00, 0x00];
        self.usb.bulk_write(read_req, IO_TIMEOUT).await?;
        self.usb.bulk_read(4, IO_TIMEOUT).await?;
        Ok(())
    }

    /// Tunes the receiver to a Band III channel.
    ///
    /// `frequency_khz_raw` matches the historical unit convention of the
    /// tuning table this resolves against: callers historically passed
    /// `table_value * 1000` here (i.e. this is effectively a Hz value, not
    /// kHz, despite the name) -- dividing it by 1000 below recovers the kHz
    /// table value. This oddity is preserved exactly because the frequency
    /// arithmetic depends on it; [`crate::tuner`] exposes a correctly-named
    /// `khz` API and does the `* 1000` at the call site.
    pub async fn set_frequency(&mut self, frequency_khz_raw: u32) -> Result<()> {
        let freq_mhz = (frequency_khz_raw / 1000) as i32; // actually a kHz value, see doc comment above.
        let mut n_idx =
            (freq_mhz - tables::DAB_CH_BAND3_START_FREQ_KHZ) / tables::DAB_CH_BAND3_STEP_FREQ_KHZ;

        if freq_mhz >= 224096 {
            n_idx += 3;
        } else if freq_mhz >= 217008 {
            n_idx += 2;
        } else if freq_mhz >= 210096 {
            n_idx += 1;
        }

        // Defensive addition (not present upstream, which has no bounds
        // check here): turn an unsupported frequency into a catchable error
        // instead of an out-of-bounds table access.
        let idx = usize::try_from(n_idx)
            .ok()
            .filter(|&i| i < tables::NUM_DAB_ENSEMBLES)
            .ok_or(Error::UnsupportedFrequency {
                khz: frequency_khz_raw / 1000,
            })?;

        let adc_clk_freq_type = tables::ADC_CLK_TYPE_TBL_DAB_B3[idx];

        if self.changed_adc_clock(adc_clk_freq_type).await? {
            self.switch_page(RegisterPage::Rf).await?;

            let pll_nf = tables::PLL_NF_DAB_BAND3[idx];
            self.set_register(0x23, ((pll_nf >> 22) & 0xFF) as u8)
                .await?;
            self.set_register(0x24, ((pll_nf >> 14) & 0xFF) as u8)
                .await?;
            self.set_register(0x25, ((pll_nf >> 5) & 0xFF) as u8)
                .await?;
            self.set_register(0x26, (((pll_nf & 0x0000003F) << 2) | 1) as u8)
                .await?;

            tokio::time::sleep(Duration::from_millis(500)).await;

            self.set_register(0x20, 0x00).await?;

            let _verify_byte_15 = self.read_register(0x15).await?;
            let _verify_byte_12 = self.read_register(0x12).await?;
            let _verify_byte_13 = self.read_register(0x13).await?;
            let _verify_byte_14 = self.read_register(0x14).await?;

            self.switch_page(RegisterPage::Fm).await?;
            self.set_register(0x10, 0x48).await?;
            self.set_register(0x10, 0xC9).await?;
        }
        Ok(())
    }

    /// Issues a soft reset of the demodulator (used after tuning to a new
    /// frequency).
    pub async fn soft_reset(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Fm).await?;

        self.set_register(0x10, 0x48).await?;
        // there should be a brief pause here before re-enabling
        self.set_register(0x10, 0xC9).await?;
        Ok(())
    }

    /// Enables FIC memory auto-clear with CRC check included.
    pub async fn setup_memory_fic(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Dd).await?;

        // auto user clr, get fic  CRC 2byte including[4]
        self.set_register(0x46, 0x14).await?;
        self.set_register(0x46, 0x16).await?;
        Ok(())
    }

    /// Sets the MSC memory interrupt threshold.
    pub async fn setup_msc_threshold(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Dd).await?;

        // Threshold at 0x100 byte
        self.set_register(0x56, 0x01).await?;
        self.set_register(0x57, 0x00).await?;

        self.switch_page(RegisterPage::Dd).await?;
        Ok(())
    }

    /// Reads the combined OFDM/FEC lock status.
    pub async fn get_lock_status(&mut self) -> Result<u8> {
        self.switch_page(RegisterPage::Dd).await?;

        let lock_state = self.read_register(0x37).await?;
        let mut lock_st = 0u8;

        if lock_state & 0x01 != 0 {
            lock_st = RTV_DAB_OFDM_LOCK_MASK;
        }

        let lock_state = self.read_register(0xFB).await?;
        if lock_state & 0x03 == 0x03 {
            lock_st |= RTV_DAB_FEC_LOCK_MASK;
        }

        Ok(lock_st)
    }

    /// Accumulates the MSC CER (Channel Error Rate) period counter and
    /// buckets it against the antenna-level table into a smoothed 0..=6
    /// signal level. Returns `None` when OFDM isn't locked (in which case
    /// the smoothed level is left unchanged).
    pub async fn get_antenna_level(&mut self) -> Result<Option<u8>> {
        self.switch_page(RegisterPage::Dd).await?;

        let lock_stat = self.read_register(0x37).await? & 0x01 != 0;
        let cer_period_cnt: u32;
        let (rcnt3, rcnt2, rcnt1, rcnt0);
        if lock_stat {
            // MSC CER period counter for accumulation
            let b3 = self.read_register(0x88).await?;
            let b2 = self.read_register(0x89).await?;
            let b1 = self.read_register(0x8A).await?;
            let b0 = self.read_register(0x8B).await?;
            cer_period_cnt =
                ((b3 as u32) << 24) | ((b2 as u32) << 16) | ((b1 as u32) << 8) | b0 as u32;

            rcnt3 = self.read_register(0x8C).await?;
            rcnt2 = self.read_register(0x8D).await?;
            rcnt1 = self.read_register(0x8E).await?;
            rcnt0 = self.read_register(0x8F).await?;
        } else {
            return Ok(None);
        }

        let fec_sync = (self.read_register(0xD7).await? >> 4) & 0x1;

        let ret_val: u32 = if cer_period_cnt != 0 {
            let cer_cnt = ((rcnt3 as u32) << 24)
                | ((rcnt2 as u32) << 16)
                | ((rcnt1 as u32) << 8)
                | rcnt0 as u32;
            if cer_cnt <= 4000 {
                0
            } else {
                let v = (cer_cnt * 1000 / cer_period_cnt) * 10;
                if v > 1200 {
                    2000
                } else {
                    v
                }
            }
        } else {
            2000
        };

        if fec_sync == 0 || ret_val == 2000 {
            // This directly re-selects the FEC/MSC register page (0x09) by
            // writing the page-select register, equivalent to
            // switch_page(RegisterPage::Dd) but written out explicitly here
            // to match the original register sequence.
            self.set_register(0x03, 0x09).await?;
            self.set_register(0x46, 0x1E).await?;
            self.set_register(0x35, 0x01).await?;
            self.set_register(0x46, 0x16).await?;
        }

        // Bucket into a smoothed antenna level, moving at most one step per
        // call towards the newly measured bucket.
        let mut cur_level: usize = 0;
        loop {
            if ret_val >= tables::ANT_LVL_TBL[cur_level] {
                // Use equal for CER 0
                break;
            }
            cur_level += 1;
            if cur_level == tables::DAB_MAX_NUM_ANTENNA_LEVEL {
                break;
            }
        }

        let mut prev_level = self.prev_antenna_lvl as i32;
        if cur_level as i32 != prev_level {
            if (cur_level as i32) < prev_level {
                prev_level -= 1;
            } else {
                prev_level += 1;
            }
            self.prev_antenna_lvl = prev_level as u8;
        }

        Ok(Some(self.prev_antenna_lvl))
    }

    /// Reads the combined interrupt status register (`RaonTunerInput::readData`'s
    /// `int_type_val1`) -- test the result against the `*_E_INT`/`*_E_OVER_FLOW`/
    /// `*_E_UNDER_FLOW` constants above.
    pub async fn read_interrupt_status(&mut self) -> Result<u8> {
        self.switch_page(RegisterPage::Dd).await?;
        self.read_register(INT_E_STATL).await
    }

    /// Requests and reads FIC memory (`RaonTunerInput::readFicData`).
    /// Returns the raw FIC bytes with the device's own 4-byte reply header
    /// stripped (matching the reference dropping `reFicRet.begin()..+4`),
    /// or an empty `Vec` if the device replied with 4 bytes or fewer.
    pub async fn read_fic_data(&mut self) -> Result<Vec<u8>> {
        self.switch_page(RegisterPage::Fic).await?;

        let request = vec![0x22, 0x00, 0x80, 0x01, 0x10];
        self.usb.bulk_write(request, IO_TIMEOUT).await?;
        let reply = self.usb.bulk_read(400, IO_TIMEOUT).await?;

        // FIC interrupt status clear.
        self.switch_page(RegisterPage::Dd).await?;
        self.set_register(INT_E_UCLRL, UCLR_FIC_INT).await?;

        if reply.len() > 4 {
            Ok(reply[4..].to_vec())
        } else {
            Ok(Vec::new())
        }
    }

    /// Requests and reads MSC1 memory (`RaonTunerInput::readMscData`).
    /// Returns the raw MSC bytes with the device's own 4-byte reply header
    /// stripped, or an empty `Vec` if the device replied with 4 bytes or
    /// fewer.
    ///
    /// This is a bare register-level read only -- there is no
    /// subchannel-open/DAB+ demux wired up to consume its output yet
    /// (superframe sync, Reed-Solomon decode, AU splitting: a later stage
    /// of the port). Calling it without first opening a subchannel
    /// (`openSubChannel`'s equivalent, not yet ported) will not produce
    /// meaningful data -- MSC1 data-ready interrupts shouldn't fire at all
    /// until a subchannel is open.
    pub async fn read_msc_data(&mut self) -> Result<Vec<u8>> {
        self.switch_page(RegisterPage::Msc1).await?;

        let request = vec![0x22, 0x00, 0x00, 0x01, 0x10];
        self.usb.bulk_write(request, IO_TIMEOUT).await?;
        let reply = self.usb.bulk_read(4096, IO_TIMEOUT).await?;

        self.switch_page(RegisterPage::Dd).await?;
        self.set_register(INT_E_UCLRL, UCLR_MSC1_INT).await?;

        if reply.len() > 4 {
            Ok(reply[4..].to_vec())
        } else {
            Ok(Vec::new())
        }
    }

    /// Clears the MSC memory buffer after an overrun/underrun
    /// (`RaonTunerInput::clearMscBuffer`).
    pub async fn clear_msc_buffer(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Dd).await?;
        self.set_register(0x48, 0x00).await?;
        self.set_register(0x48, 0x05).await?;
        self.set_register(INT_E_UCLRL, UCLR_MSC1_INT).await?;
        Ok(())
    }

    /// Clears and reconfigures the MSC memory ahead of opening a
    /// subchannel (`RaonTunerInput::clearAndSetupMscMemory`).
    ///
    /// Not to be confused with [`Hal::clear_msc_buffer`]
    /// (`clearMscBuffer`), a *different* function with an almost-identical
    /// but not-quite-identical body (the same two `0x48` writes, but
    /// without this one's trailing `INT_E_UCLRL` write) -- easy to
    /// conflate since both a plain-text read and the reference's own
    /// naming invite it, but the two are called from different places for
    /// different reasons and neither substitutes for the other. This one
    /// is only ever called immediately before `openSubChannel` in the
    /// reference (`setService`'s `clearAndSetupMscMemory();
    /// openSubChannel(...)` pair) -- folded into [`Hal::open_subchannel`]
    /// itself here to match that as its one real call site.
    async fn clear_and_setup_msc_memory(&mut self) -> Result<()> {
        self.switch_page(RegisterPage::Dd).await?;
        self.set_register(0x48, 0x00).await?;
        self.set_register(0x48, 0x05).await?;
        Ok(())
    }

    /// Opens `subchan_id` for MSC1 data delivery
    /// (`RaonTunerInput::clearAndSetupMscMemory` + `openSubChannel`, called
    /// as that exact pair at the reference's one call site -- see
    /// [`Hal::clear_and_setup_msc_memory`]'s doc comment). Tracking which
    /// subchannel is currently open (the reference's `m_currentSubchanId`)
    /// is left to the caller (`tuner.rs`) -- this is a bare register-level
    /// primitive.
    pub async fn open_subchannel(&mut self, subchan_id: u8) -> Result<()> {
        self.clear_and_setup_msc_memory().await?;

        self.switch_page(RegisterPage::Dd).await?;
        self.set_register(INT_E_UCLRL, 0x70).await?;

        self.switch_page(RegisterPage::Host).await?;
        self.set_register(0x62, 0x00).await?;

        self.switch_page(RegisterPage::Dd).await?;
        self.set_register(0x3A, 0x80 | subchan_id).await?;
        Ok(())
    }

    /// Closes `subchan_id` (`RaonTunerInput::closeSubchannel`).
    pub async fn close_subchannel(&mut self, subchan_id: u8) -> Result<()> {
        self.switch_page(RegisterPage::Dd).await?;
        // TODO(unverified): both this and `open_subchannel` write 0x70 to
        // the same register address as INT_E_UCLRL (0x35, see its doc
        // comment above), described in the reference only as "MSC1
        // Interrupt status clear". That value fits neither INT_E_UCLRL's
        // own documented 3-bit layout (bits 0-2) nor a literal reuse of
        // INT_E_STATL's bit numbering it happens to resemble (bits 4-6 =
        // MSC1_E_INT|MSC1_E_UNDER_FLOW|MSC1_E_OVER_FLOW = 0x70). Kept as
        // the literal register/value pair the source has, without
        // asserting which (if either) interpretation is correct -- see
        // PORT_CHECKLIST.md's Phase 3 entry for this same open question.
        self.set_register(INT_E_UCLRL, 0x70).await?;

        self.switch_page(RegisterPage::Host).await?;
        self.set_register(0x62, 0x00).await?;

        self.switch_page(RegisterPage::Dd).await?;
        self.set_register(0x3A, subchan_id).await?;
        Ok(())
    }

    /// Access to the underlying USB device, for callers that need to issue
    /// raw bulk transfers outside the register-write framing (e.g. the
    /// future FIC/MSC memory read loop).
    pub fn usb_mut(&mut self) -> &mut UsbDevice {
        &mut self.usb
    }
}
