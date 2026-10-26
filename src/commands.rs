use std::sync::OnceLock;

use qtbridge::invoke_method;
use tokio::sync::mpsc;

use crate::store::settings_store::reply;
use crate::{audio, screen};

// Work the UI hands to the background so it never waits on hardware. Results come back through
// store::settings_store::reply, which queues them onto Qt's event loop.
pub enum Commands {
    RefreshAll,
    RefreshDevices,
    SetBrightness(u32),
    // 0-100 as shown in the UI, see ui_to_system_volume
    SetVolume(u8),
    SetDefaultSoundDevice(String),
    SetDefaultInputDevice(String),
}

// The system volume tops out at 65 on the target hardware
const MAX_SYSTEM_VOLUME: f64 = 65.0;

// Unbounded: a command is never refused or dropped, and the worker handles them strictly in the
// order they were sent.
static COMMANDS: OnceLock<mpsc::UnboundedSender<Commands>> = OnceLock::new();

// Never blocks
pub fn send(command: Commands) {
    let Some(commands) = COMMANDS.get() else {
        eprintln!("command loop not started");
        return;
    };
    if commands.send(command).is_err() {
        eprintln!("command loop stopped");
    }
}

pub fn start() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    COMMANDS.set(tx).unwrap_or_else(|_| panic!("commands::start called twice"));

    crate::runtime::spawn(async move {
        while let Some(command) = rx.recv().await {
            handle(command).await;
        }
    });
}

fn ui_to_system_volume(ui: u8) -> u8 {
    (f64::from(ui) * MAX_SYSTEM_VOLUME / 100.0).round() as u8
}

fn system_to_ui_volume(system: u8) -> u8 {
    (f64::from(system) * 100.0 / MAX_SYSTEM_VOLUME).round().min(100.0) as u8
}

// The audio calls are blocking (pactl, WASAPI, ...), so they run off the async worker threads
async fn blocking<T: Send + 'static>(call: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(call).await.map_err(|e| e.to_string())
}

fn log<T>(what: &str, result: Result<T, String>) -> Option<T> {
    result.inspect_err(|e| eprintln!("{what}: {e}")).ok()
}

async fn handle(command: Commands) {
    match command {
        Commands::RefreshAll => {
            refresh_brightness().await;
            refresh_volume().await;
            refresh_default_devices().await;
            refresh_devices().await;
        }
        Commands::RefreshDevices => refresh_devices().await,
        Commands::SetBrightness(value) => {
            log("set brightness", screen::set_brightness(value).await);
        }
        Commands::SetVolume(ui) => {
            // 0 mutes. The two calls don't depend on each other, so they run side by side
            let mute = ui == 0;
            let volume = if mute { 0 } else { ui_to_system_volume(ui) };
            let (mute_result, volume_result) = tokio::join!(
                blocking(move || audio::set_mute(mute)),
                blocking(move || audio::set_current_volume(volume)),
            );
            log("set mute", mute_result.and_then(|r| r));
            log("set volume", volume_result.and_then(|r| r));
        }
        Commands::SetDefaultSoundDevice(name) => {
            log("set default sound device", audio::set_default_device(name).await);
        }
        Commands::SetDefaultInputDevice(name) => {
            let result = blocking(move || audio::set_default_input_device(&name)).await.and_then(|r| r);
            log("set default input device", result);
        }
    }
}

async fn refresh_brightness() {
    if let Some(value) = log("get brightness", screen::get_current_brightness().await) {
        reply(|store| {
            invoke_method!(store, "brightness_loaded", value);
        });
    }
}

async fn refresh_volume() {
    let result = blocking(|| audio::get_current_volume()).await.and_then(|r| r);
    if let Some(system) = log("get volume", result) {
        let value = system_to_ui_volume(system);
        reply(|store| {
            invoke_method!(store, "volume_loaded", value);
        });
    }
}

async fn refresh_default_devices() {
    let sound = blocking(|| audio::get_default_device_name()).await.and_then(|r| r);
    if let Some(name) = log("get default sound device", sound) {
        reply(|store| {
            invoke_method!(store, "default_sound_device_loaded", name);
        });
    }
    let input = blocking(|| audio::get_default_input_device_name()).await.and_then(|r| r);
    if let Some(name) = log("get default input device", input) {
        reply(|store| {
            invoke_method!(store, "default_input_device_loaded", name);
        });
    }
}

async fn refresh_devices() {
    if let Some(devices) = log("list audio devices", audio::get_audio_devices().await) {
        reply(|store| {
            invoke_method!(store, "audio_devices_loaded", devices);
        });
    }
    let input = blocking(|| audio::get_input_devices()).await.and_then(|r| r);
    if let Some(devices) = log("list input devices", input) {
        reply(|store| {
            invoke_method!(store, "input_devices_loaded", devices);
        });
    }
}
