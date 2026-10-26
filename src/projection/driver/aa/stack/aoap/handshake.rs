//! AOAP handshake — switch a stock Android phone into accessory mode.
//! Spec: <https://source.android.com/docs/core/interaction/accessories/aoa>

use std::time::Duration;

use nusb::transfer::{ControlIn, ControlOut, ControlType, Recipient, TransferError};
use nusb::{Device, DeviceInfo};
#[cfg(not(target_os = "linux"))]
use nusb::Interface;

use super::constants::{
    ACCESSORY_PIDS, AOAP_DESCRIPTION, AOAP_MANUFACTURER, AOAP_MODEL, AOAP_SERIAL, AOAP_URI,
    AOAP_VERSION, GOOGLE_VID, REQ_GET_PROTOCOL, REQ_SEND_STRING, REQ_START, STRING_DESCRIPTION,
    STRING_MANUFACTURER, STRING_MODEL, STRING_SERIAL, STRING_URI, STRING_VERSION,
};

const TRANSFER_TIMEOUT: Duration = Duration::from_millis(2_000);

#[derive(Debug)]
pub enum HandshakeError {
    #[cfg(not(target_os = "linux"))]
    NoClaimableInterface,
    Transfer(TransferError),
    ProtocolTooOld(u16),
    ShortProtocolReply,
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(not(target_os = "linux"))]
            HandshakeError::NoClaimableInterface => {
                write!(f, "AOAP: no claimable interface for control transfers")
            }
            HandshakeError::Transfer(e) => write!(f, "AOAP control transfer failed: {e}"),
            HandshakeError::ProtocolTooOld(v) => {
                write!(f, "AOAP protocol version {v} not supported by device")
            }
            HandshakeError::ShortProtocolReply => write!(f, "AOAP getProtocol returned no data"),
        }
    }
}

impl std::error::Error for HandshakeError {}

impl From<TransferError> for HandshakeError {
    fn from(e: TransferError) -> Self {
        HandshakeError::Transfer(e)
    }
}

pub fn is_accessory_mode(info: &DeviceInfo) -> bool {
    println!("Checking if device is in accessory mode: {:?}", info);
    info.vendor_id() == GOOGLE_VID && ACCESSORY_PIDS.contains(&info.product_id())
}

// Where the handshake's control transfers go. On Linux they go straight to the device, which
// needs no claimed interface: another process (an adb server, a kernel driver) holding all of the
// phone's interfaces can't get in the way. Elsewhere nusb routes a device-recipient control
// transfer through any claimed interface and errors if none is claimed, so one gets claimed.
enum Control {
    #[cfg(target_os = "linux")]
    Device(Device),
    #[cfg(not(target_os = "linux"))]
    Interface(Interface),
}

impl Control {
    #[cfg(target_os = "linux")]
    async fn open(device: &Device) -> Result<Self, HandshakeError> {
        Ok(Control::Device(device.clone()))
    }

    // The claim need not be interface 0, which on macOS the kernel driver (MTP/PTP) may hold —
    // claim the first interface that is actually claimable (e.g. the vendor one).
    #[cfg(not(target_os = "linux"))]
    async fn open(device: &Device) -> Result<Self, HandshakeError> {
        let interface_numbers: Vec<u8> = device
            .active_configuration()
            .map(|config| config.interfaces().map(|i| i.interface_number()).collect())
            .unwrap_or_default();

        for number in interface_numbers {
            if let Ok(interface) = device.claim_interface(number).await {
                return Ok(Control::Interface(interface));
            }
        }

        Err(HandshakeError::NoClaimableInterface)
    }

    async fn control_in(&self, data: ControlIn, timeout: Duration) -> Result<Vec<u8>, TransferError> {
        match self {
            #[cfg(target_os = "linux")]
            Control::Device(device) => device.control_in(data, timeout).await,
            #[cfg(not(target_os = "linux"))]
            Control::Interface(interface) => interface.control_in(data, timeout).await,
        }
    }

    async fn control_out(&self, data: ControlOut<'_>, timeout: Duration) -> Result<(), TransferError> {
        match self {
            #[cfg(target_os = "linux")]
            Control::Device(device) => device.control_out(data, timeout).await,
            #[cfg(not(target_os = "linux"))]
            Control::Interface(interface) => interface.control_out(data, timeout).await,
        }
    }
}

async fn get_protocol(control: &Control) -> Result<u16, HandshakeError> {
    let data = control
        .control_in(
            ControlIn {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: REQ_GET_PROTOCOL,
                value: 0,
                index: 0,
                length: 2,
            },
            TRANSFER_TIMEOUT,
        )
        .await?;

    if data.len() < 2 {
        return Err(HandshakeError::ShortProtocolReply);
    }
    Ok(u16::from_le_bytes([data[0], data[1]]))
}

async fn send_string(control: &Control, index: u16, value: &str) -> Result<(), HandshakeError> {
    let mut data = value.as_bytes().to_vec();
    data.push(0);
    control
        .control_out(
            ControlOut {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: REQ_SEND_STRING,
                value: 0,
                index,
                data: &data,
            },
            TRANSFER_TIMEOUT,
        )
        .await?;
    Ok(())
}

async fn start_accessory(control: &Control) -> Result<(), HandshakeError> {
    control
        .control_out(
            ControlOut {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: REQ_START,
                value: 0,
                index: 0,
                data: &[],
            },
            TRANSFER_TIMEOUT,
        )
        .await?;
    Ok(())
}

/// Runs the AOAP handshake against a phone that isn't in accessory mode yet. On success the
/// phone disconnects and re-enumerates under one of `ACCESSORY_PIDS` — the caller is responsible
/// for waiting for that reattach (see `transport::usb_aoap_bridge`).
pub async fn run_aoap_handshake(device: &Device) -> Result<(), HandshakeError> {
    let control = Control::open(device).await?;

    let proto = get_protocol(&control).await?;
    if proto < 1 {
        return Err(HandshakeError::ProtocolTooOld(proto));
    }

    send_string(&control, STRING_MANUFACTURER, AOAP_MANUFACTURER).await?;
    send_string(&control, STRING_MODEL, AOAP_MODEL).await?;
    send_string(&control, STRING_DESCRIPTION, AOAP_DESCRIPTION).await?;
    send_string(&control, STRING_VERSION, AOAP_VERSION).await?;
    send_string(&control, STRING_URI, AOAP_URI).await?;
    send_string(&control, STRING_SERIAL, AOAP_SERIAL).await?;

    start_accessory(&control).await?;
    Ok(())
}
