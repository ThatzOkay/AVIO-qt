use std::sync::mpsc as std_mpsc;
use std::sync::{OnceLock};
use std::thread;
use std::time::Duration;

#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("shim.h");

        type WelleIoBridge;
        type RadioReceiver;
        type mot_file_t;

        fn new_welle_io_bridge() -> UniquePtr<WelleIoBridge>;
        fn ping(self: &WelleIoBridge) -> i32;
        #[allow(dead_code)]
        unsafe fn setReceiver(self: Pin<&mut WelleIoBridge>, receiver: *mut RadioReceiver);
        #[allow(dead_code)]
        fn setSnrCallback(self: Pin<&mut WelleIoBridge>, callback: extern "C" fn(f32));
        #[allow(dead_code)]
        fn setSignalPresenceCallback(self: Pin<&mut WelleIoBridge>, callback: extern "C" fn(bool));
        #[allow(dead_code)]
        fn setServiceDetectedCallback(self: Pin<&mut WelleIoBridge>, callback: extern "C" fn(u32, String));
        #[allow(dead_code)]
        fn setNewAudioCallback(self: Pin<&mut WelleIoBridge>, callback: extern "C" fn(Vec<i16>, i32, bool));
        #[allow(dead_code)]
        fn setNewDynamicLabelCallback(self: Pin<&mut WelleIoBridge>, callback: extern "C" fn(String));
            
        #[allow(dead_code)]
        fn setMotCallback(self: Pin<&mut WelleIoBridge>, callback: extern "C" fn(motfile: &mot_file_t));
    }
}

pub fn demo_ping() -> i32 {
    let bridge = ffi::new_welle_io_bridge();
    bridge.ping()
}


#[allow(dead_code)]
enum DeviceCmd {}

#[allow(dead_code)]
const DEVICE_CMD_TIMEOUT: Duration = Duration::from_secs(2);

#[allow(dead_code)]
static DEVICE_CMD_TX: OnceLock<std_mpsc::Sender<DeviceCmd>> = OnceLock::new();

#[allow(dead_code)]
fn device_cmd_tx() -> &'static std_mpsc::Sender<DeviceCmd> {
    DEVICE_CMD_TX.get_or_init(|| {
        let (tx, rx) = std_mpsc::channel::<DeviceCmd>();
        thread::spawn(move || device_worker_loop(rx));
        tx
    })
}

#[allow(dead_code)]
fn device_worker_loop(rx: std_mpsc::Receiver<DeviceCmd>) {
    //let mut dev: Option<ffi::UniquePtr<ffi::WelleIoBridge>> = None;
    // DeviceCmd has no variants yet - the empty match makes clippy think the
    // loop can't iterate. Will go away once DAB commands are added below.
    #[allow(clippy::never_loop)]
    while let Ok(cmd) = rx.recv() {
        match cmd {
        }
    }
}