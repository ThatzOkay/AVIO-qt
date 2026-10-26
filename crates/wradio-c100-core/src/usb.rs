//! Async USB transport for the wRadio C100 dongle.
//!
//! A thin wrapper over `nusb` bulk transfers, giving the register protocol
//! layer in `hal.rs` a couple of simple `bulk_write`/`bulk_read` calls to
//! build on, instead of dealing with endpoint objects and transfer
//! completions directly. `nusb` is pure-Rust and async-native, so no system
//! `libusb` dependency is needed.
//!
//! The device exposes its register protocol over one bulk OUT and one bulk
//! IN endpoint on a single USB interface, which is claimed at open time.
//!
// TODO(unverified): claiming "the first interface of the active
// configuration" is an inference -- nothing in the reference sources
// enumerates the interface number explicitly (they open the raw device
// handle and use the two endpoints directly without ever claiming an
// interface at all). If this device ever turns out to expose more than one
// USB interface, this needs to target the right one explicitly.

use std::time::Duration;

use nusb::transfer::{Buffer, Bulk, In, Out};
use nusb::{Device, Interface};

use crate::error::{Error, Result};

/// USB vendor/product ID this device enumerates as.
pub const VENDOR_ID: u16 = 0x16c0;
pub const PRODUCT_ID: u16 = 0x05dc;

/// Bulk OUT endpoint used for register writes and read requests.
const ENDPOINT_OUT: u8 = 0x02;
/// Bulk IN endpoint used for register/FIC/MSC read replies.
const ENDPOINT_IN: u8 = 0x82;

/// Owns the claimed USB interface and the two bulk endpoints the register
/// protocol and FIC/MSC memory reads run over.
pub struct UsbDevice {
    // Kept alive alongside the endpoints: dropping the `Interface` releases
    // the claim, and both endpoints borrow from it internally.
    _interface: Interface,
    ep_out: nusb::Endpoint<Bulk, Out>,
    ep_in: nusb::Endpoint<Bulk, In>,
    in_max_packet_size: usize,
}

impl UsbDevice {
    /// Finds and opens the first attached device matching [`VENDOR_ID`]/[`PRODUCT_ID`].
    pub async fn open_first() -> Result<Self> {
        let device_info = nusb::list_devices()
            .await?
            .find(|d| d.vendor_id() == VENDOR_ID && d.product_id() == PRODUCT_ID)
            .ok_or(Error::DeviceNotFound)?;

        let device: Device = device_info.open().await?;
        Self::from_device(device).await
    }

    async fn from_device(device: Device) -> Result<Self> {
        let config = device
            .active_configuration()
            .map_err(|_| Error::DeviceNotFound)?;
        let interface_number = config
            .interfaces()
            .next()
            .ok_or(Error::DeviceNotFound)?
            .interface_number();

        let interface = device.claim_interface(interface_number).await?;

        let ep_out = interface.endpoint::<Bulk, Out>(ENDPOINT_OUT)?;
        let ep_in = interface.endpoint::<Bulk, In>(ENDPOINT_IN)?;
        let in_max_packet_size = ep_in.max_packet_size().max(1);

        Ok(Self {
            _interface: interface,
            ep_out,
            ep_in,
            in_max_packet_size,
        })
    }

    /// Writes `data` to the OUT endpoint and returns the number of bytes
    /// actually transferred, cancelling the transfer if `timeout` elapses.
    pub async fn bulk_write(&mut self, data: Vec<u8>, timeout: Duration) -> Result<usize> {
        self.ep_out.submit(Buffer::from(data));
        let completion = match tokio::time::timeout(timeout, self.ep_out.next_complete()).await {
            Ok(completion) => completion,
            Err(_) => {
                self.ep_out.cancel_all();
                self.ep_out.next_complete().await
            }
        };
        let actual_len = completion.actual_len;
        check_transfer_status(completion.status)?;
        Ok(actual_len)
    }

    /// Reads up to `max_len` bytes from the IN endpoint, cancelling the
    /// transfer if `timeout` elapses. The returned `Vec` is truncated to the
    /// number of bytes the device actually sent.
    ///
    /// The underlying transport requires an IN transfer's requested length
    /// to be a nonzero multiple of the endpoint's max packet size (it uses a
    /// short packet to detect end-of-transfer) -- so the actual request is
    /// rounded up to that; a reply shorter than `max_len` still ends the
    /// read at the right point via its own short final packet.
    pub async fn bulk_read(&mut self, max_len: usize, timeout: Duration) -> Result<Vec<u8>> {
        let requested = max_len.max(self.in_max_packet_size);
        let requested = requested.div_ceil(self.in_max_packet_size) * self.in_max_packet_size;

        self.ep_in.submit(Buffer::new(requested));
        let completion = match tokio::time::timeout(timeout, self.ep_in.next_complete()).await {
            Ok(completion) => completion,
            Err(_) => {
                self.ep_in.cancel_all();
                self.ep_in.next_complete().await
            }
        };
        check_transfer_status(completion.status)?;
        let mut data = completion.buffer.to_vec();
        data.truncate(max_len.min(data.len()));
        Ok(data)
    }
}

/// Maps a transfer's status to this crate's error type, distinguishing a
/// device disconnect (`Error::DeviceDisconnected`, meaningful to callers
/// deciding whether to attempt a reconnect) from every other transfer
/// failure (`Error::Transfer`, treated as an ordinary I/O error).
fn check_transfer_status(
    status: std::result::Result<(), nusb::transfer::TransferError>,
) -> Result<()> {
    match status {
        Ok(()) => Ok(()),
        Err(nusb::transfer::TransferError::Disconnected) => Err(Error::DeviceDisconnected),
        Err(e) => Err(Error::Transfer(e)),
    }
}
