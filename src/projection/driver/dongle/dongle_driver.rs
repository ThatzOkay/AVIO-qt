// Placeholder for the CarLinkit dongle port (ported from the old WebUSB implementation): USBDevice/USBEndpoint
// there were WebUSB types, which nusb::Device covers directly. A USBEndpoint there is only ever
// used for its `endpointNumber`, so it maps to a bare endpoint address here.
pub type UsbDevice = nusb::Device;

#[derive(Clone, Copy, Debug)]
pub struct UsbEndpoint {
    pub address: u8,
}

pub enum AndroidWorkMode {
    Off = 0,
    AndroidAuto = 1,
    CarLife = 2,
    AndroidMirror = 3,
    Search = 7,
}

struct DongleDriver {
    device: Option<UsbDevice>,
    in_ep: Option<UsbEndpoint>,
    out_ep: Option<UsbEndpoint>,
    iface_number: Option<u8>,
    error_count: u32,
    closing: bool,
    started: bool,
    reader_active: bool,
    close_fn: Option<Box<dyn FnOnce() + Send>>,
    dongle_fw_version: Option<String>,
}
