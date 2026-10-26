use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_lite::StreamExt;
use qtbridge::{QmlObject, invoke_method, qobject};
use regex::Regex;
use rtl_sdr::detect_rtl_sdr;
use tokio::sync::Notify;

use crate::projection::driver::aa::stack::aoap::handshake::is_accessory_mode;
use crate::radio::{self, hardware::{DabBackendKind, RadioHardware}};
use crate::runtime;
use crate::store::{Remote, status_store};
use crate::usb::constants::is_wradio_c100;
use crate::usb::udev_rule::phone_vendor_ids_from_udev_template;

const NON_PHONE_INTERFACE_CLASSES: [u8; 10] = [
    0x01, // Audio
    0x02, // Communications
    0x03, // HID
    0x07, // Printer
    0x08, // Mass Storage
    0x09, // Hub
    0x0a, // CDC-Data
    0x0b, // Smart Card
    0x0e, // Video
    0xe0, // Wireless
];

const PHONE_REENUM_SUPRESS_MS: u32 = 2_500;

#[allow(dead_code)]
static DISCONNECT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)not[\s_-]?found|no[\s_-]?device|disconnect").unwrap());

// Dongle (wireless AA dongle) support, not yet wired up.
#[allow(dead_code)]
struct DongleUsbBasics {
    vendor_id: u16,
    product_id: u16,
    usb_fw_version: String,
}

static REMOTE: Remote = Remote::new();
static STARTED: AtomicBool = AtomicBool::new(false);
static STOP: Notify = Notify::const_new();

// Qt owns this object, so the hotplug loop can't live in it. It runs as a task on the tokio runtime
// and owns its state (`UsbMonitor`); this object only forwards what the loop reports to QML.
#[derive(Default)]
pub struct UsbService {}

#[qobject(Singleton)]
impl UsbService {
    // kind is "attach" or "detach"
    #[qsignal]
    fn usb_event(&mut self, kind: String, vendor_id: u32, product_id: u32, device_name: String);

    // Call once from QML on startup (after StatusStore.init(), the monitor reports radio hardware
    // through it): registers for events and starts watching USB
    #[qslot]
    fn init(&self) {
        REMOTE.set(self.get_qml_method_invoker());
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        runtime::spawn(UsbMonitor::new().run());
    }

    #[qslot]
    fn usb_event_received(&mut self, kind: String, vendor_id: u32, product_id: u32, device_name: String) {
        self.usb_event(kind, vendor_id, product_id, device_name);
    }
}

// Stops the hotplug loop
pub fn stop() {
    STOP.notify_one();
}

struct UsbMonitor {
    last_dongle_state: bool,
    last_phone_state: bool,
    last_radio_hardware: Option<RadioHardware>,
    phone_suspend_until: u32,
    connected_phone_device: Option<nusb::DeviceInfo>,
    known_devices: std::collections::HashMap<nusb::DeviceId, nusb::DeviceInfo>,
}

impl UsbMonitor {
    fn new() -> Self {
        UsbMonitor {
            last_dongle_state: false,
            last_phone_state: false,
            last_radio_hardware: None,
            phone_suspend_until: 0,
            connected_phone_device: None,
            known_devices: std::collections::HashMap::new(),
        }
    }

    async fn run(mut self) {
        self.init().await;

        let mut watcher = match nusb::watch_devices() {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[UsbService] Failed to watch USB devices: {:?}", e);
                return;
            }
        };

        loop {
            let event = tokio::select! {
                () = STOP.notified() => {
                    println!("[UsbService] Received stop signal, stopping USB service.");
                    return;
                }
                event = watcher.next() => event,
            };
            match event {
                Some(nusb::hotplug::HotplugEvent::Connected(device)) => self.on_connect(device).await,
                Some(nusb::hotplug::HotplugEvent::Disconnected(device_id)) => {
                    self.on_disconnect(device_id).await
                }
                None => return,
            }
        }
    }

    async fn on_connect(&mut self, device: nusb::DeviceInfo) {
        let is_dongle = self.is_dongle(&device);
        println!("[UsbService] Device connected: {:?}", device);
        self.known_devices.insert(device.id(), device.clone());
        self.refresh_radio_hardware().await;
        if !is_dongle {
            if !self.last_phone_state
                && !self.is_phone_suspend_window()
                && self.is_phone_candidate(&device)
            {
                self.mark_phone_attached(device.clone());
            }
            self.broadcast_generic_usb_event("attach", device);
        }
    }

    async fn on_disconnect(&mut self, device_id: nusb::DeviceId) {
        let device = match self.known_devices.remove(&device_id) {
            Some(d) => d,
            None => {
                eprintln!(
                    "[UsbService] Device disconnected but not found in tracked devices: {:?}",
                    device_id
                );
                return;
            }
        };

        self.refresh_radio_hardware().await;
        let is_dongle = self.is_dongle(&device);
        println!("[UsbService] Device disconnected: {:?}", device);
        if !is_dongle {
            if self.is_same_phone_device(&device) {
                self.mark_phone_detached(&device);
            }
            self.broadcast_generic_usb_event("detach", device);
        }
    }

    /// Re-derives which radio hardware is plugged in and reports it when it changed. An RTL-SDR
    /// can do either mode, the WRadio C100 only DAB (and wins it).
    async fn refresh_radio_hardware(&mut self) {
        let wradio_present = self
            .known_devices
            .values()
            .any(|d| is_wradio_c100(d.vendor_id(), d.product_id()));
        // Enumerates USB, so keep it off the async worker
        let rtl_sdr_present = tokio::task::spawn_blocking(detect_rtl_sdr).await.unwrap_or(false);

        let hardware = RadioHardware::detect(wradio_present, rtl_sdr_present);

        if self.last_radio_hardware != Some(hardware) {
            self.last_radio_hardware = Some(hardware);
            radio::hardware::set(hardware);
            status_store::set_radio_hardware(hardware.fm, hardware.dab.map(DabBackendKind::name));
        }
    }

    async fn init(&mut self) {
        let devices: Vec<_> = match nusb::list_devices().await {
            Ok(devices) => devices.collect(),
            Err(e) => {
                eprintln!("[UsbService] Failed to list USB devices: {:?}", e);
                Vec::new()
            }
        };

        for device in &devices {
            self.known_devices.insert(device.id(), device.clone());
        }

        if devices.iter().any(|d| self.is_dongle(d)) {
            self.last_dongle_state = true;
            // projection mark dongle connected
            //self.notify_device_changed(device, true);
        }

        self.refresh_radio_hardware().await;
        self.scan_for_existing_phone().await;
    }

    async fn scan_for_existing_phone(&mut self) {
        if self.last_phone_state {
            return;
        }

        let devices: Vec<_> = match nusb::list_devices().await {
            Ok(devices) => devices.collect(),
            Err(e) => {
                eprintln!("[UsbService] Failed to list USB devices: {:?}", e);
                return;
            }
        };

        let accessory = devices.iter().find(|d| is_accessory_mode(d));
        if let Some(accessory) = accessory {
            println!(
                "[UsbService] Found device in accessory mode: {:?}",
                accessory
            );
            self.mark_phone_attached(accessory.clone());
        }

        println!("[UsbService] Scanning for candidate phone devices...");

        let condidate = devices.iter().find(|d| self.is_phone_candidate(d));
        if let Some(candidate) = condidate {
            println!("[UsbService] Found candidate phone device: {:?}", candidate);
            self.mark_phone_attached(candidate.clone());
        } else {
            println!("[UsbService] No candidate phone devices found.");
        }
    }

    fn is_phone_suspend_window(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u32;
        now < self.phone_suspend_until
    }

    fn mark_phone_attached(&mut self, device: nusb::DeviceInfo) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u32;
        self.last_phone_state = true;
        self.phone_suspend_until = now + PHONE_REENUM_SUPRESS_MS;

        let wired_device = device.clone();
        self.connected_phone_device = Some(device);
        runtime::spawn(async move {
            crate::projection::driver::aa::wired_driver::connect_wired(wired_device).await;
        });
    }

    fn mark_phone_detached(&mut self, _device: &nusb::DeviceInfo) {
        self.last_phone_state = false;
        self.connected_phone_device = None;
        // Deliberately NOT resetting phone_suspend_until here: AOAP re-enumeration disconnects
        // and reconnects the *same* physical phone under a different vendor/product ID pair
        // mid-handshake. Zeroing the suspend window on that disconnect let on_connect treat the
        // freshly re-enumerated accessory-mode device as a brand-new phone and spawn a second,
        // racing connect_wired against the first one's own re-enumeration wait — hence "interface
        // is busy". Leaving phone_suspend_until alone lets it expire naturally on its own timer.
        // projection.mark_phone_connected(false);
    }

    fn is_phone_candidate(&self, device: &nusb::DeviceInfo) -> bool {
        if self.is_dongle(device) {
            return false;
        }

        let class = device.class();

        if class != 0x00 && class != 0xFF {
            return false;
        }

        if self.has_only_none_phone_interfaces(device) {
            return false;
        }

        if std::env::consts::OS == "linux" {
            if let Ok(vendors) = phone_vendor_ids_from_udev_template() {
                if !vendors.contains(&device.vendor_id()) {
                    return false;
                }
            }
        }

        true
    }

    fn has_only_none_phone_interfaces(&self, device: &nusb::DeviceInfo) -> bool {
        for interface in device.interfaces() {
            if !NON_PHONE_INTERFACE_CLASSES.contains(&interface.class()) {
                return false;
            }
        }
        true
    }

    fn is_same_phone_device(&self, device: &nusb::DeviceInfo) -> bool {
        if let Some(ref connected_phone) = self.connected_phone_device {
            return connected_phone.vendor_id() == device.vendor_id()
                && connected_phone.product_id() == device.product_id();
        }
        false
    }

    fn broadcast_generic_usb_event(&self, event_type: &str, device: nusb::DeviceInfo) {
        let kind = event_type.to_string();
        let vendor_id = u32::from(device.vendor_id());
        let product_id = u32::from(device.product_id());
        let device_name = device.product_string().unwrap_or("unknown").to_string();

        REMOTE.with(|invoker| {
            invoke_method!(invoker, "usb_event_received", kind, vendor_id, product_id, device_name);
        });
    }

    #[allow(dead_code)]
    fn get_dongle_usb_basics(&self, device: &nusb::DeviceInfo) -> DongleUsbBasics {
        let device_version = device.device_version(); // u16, BCD e.g. 0x0203

        let major = (device_version >> 8) as u8;
        let minor = ((device_version >> 4) & 0xF) as u8;
        let subminor = (device_version & 0xF) as u8;

        let low_byte = (minor << 4) | subminor;
        let bcd = ((major as u16) << 8) | (low_byte as u16);

        let usb_fw_version = if bcd != 0 {
            format!("{}.{:02}", major, low_byte)
        } else {
            "Unknown".to_string()
        };

        let vendor_id = device.vendor_id();
        let product_id = device.product_id();

        DongleUsbBasics {
            vendor_id,
            product_id,
            usb_fw_version,
        }
    }

    fn is_dongle(&self, device: &nusb::DeviceInfo) -> bool {
        crate::usb::constants::is_carlinkit_dongle(
            Some(device.vendor_id()),
            Some(device.product_id()),
        )
    }

    #[allow(dead_code)]
    fn notify_reset(_notify_type: String, _ok: bool) {
        // TODO
    }

    pub async fn force_reset() {}

    pub async fn graceful_reset() {}

    #[allow(dead_code)]
    async fn reset_dongle(&mut self, dongle: nusb::DeviceInfo) -> bool {
        let device = match dongle.open().await {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[UsbService] Failed to open dongle for reset: {:?}", e);
                return false;
            }
        };

        match device.reset().await {
            Ok(_) => {
                println!("[UsbService] Dongle reset successfully.");
                // note: nusb has no explicit close(), device drops here
                true
            }
            Err(e) => {
                if DISCONNECT_RE.is_match(&e.to_string()) {
                    println!("[USB] reset triggered disconnect – treating as success");
                    return true;
                }
                eprintln!("[UsbService] Failed to reset dongle: {:?}", e);
                false
            }
        }
    }
}
