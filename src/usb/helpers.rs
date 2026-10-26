use nusb::{list_devices, DeviceInfo, MaybeFuture};

use crate::usb::constants::is_carlinkit_dongle;

pub async fn find_dongle() -> Result<Option<DeviceInfo>, Box<dyn std::error::Error>> {
    Ok(list_devices()
        .wait()?
        .find(|device| is_carlinkit_dongle(Some(device.vendor_id()), Some(device.product_id()))))
}
