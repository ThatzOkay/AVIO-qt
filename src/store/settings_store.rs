use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use qtbridge::{QmlObject, qobject};
use serde::{Deserialize, Serialize};

use super::Remote;
use crate::commands::{self, Commands};
use crate::radio::dab::DabStation;
use crate::theme::DEFAULT_SEED;

const SAVE_DEBOUNCE_MS: u64 = 500;

// Persisted settings. Unknown or missing keys fall back to the defaults, so the file can grow over time
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub master_volume: u8,
    pub default_sound_device: String,
    pub default_input_device: String,
    pub brightness: u32,
    pub dark: bool,
    pub seed: String,
    // Android Auto session
    pub ao_resolution: String,
    pub ao_framerate: String,
    pub ao_dpi: f64,
    pub ao_view_area_top: u32,
    pub ao_view_area_bottom: u32,
    pub ao_view_area_left: u32,
    pub ao_view_area_right: u32,
    pub ao_safe_area_top: u32,
    pub ao_safe_area_bottom: u32,
    pub ao_safe_area_left: u32,
    pub ao_safe_area_right: u32,
    // Window
    pub kiosk: bool,
    // "" = host/panel default. See `window::display_mode`.
    pub display_mode: String,
    // Radio: "fm" or "dab", the last mode that was open
    pub radio_mode: String,
    pub fm_frequency: u32,
    pub fm_favorites: Vec<u32>,
    pub dab_stations: Vec<DabStation>,
    pub dab_favorites: Vec<Option<DabStation>>,
    pub dab_last: Option<DabStation>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            master_volume: 0,
            default_sound_device: String::new(),
            default_input_device: String::new(),
            brightness: 0,
            dark: false,
            seed: DEFAULT_SEED.to_string(),
            ao_resolution: "1080p".to_string(),
            ao_framerate: "60".to_string(),
            ao_dpi: 160.0,
            ao_view_area_top: 0,
            ao_view_area_bottom: 0,
            ao_view_area_left: 0,
            ao_view_area_right: 0,
            ao_safe_area_top: 0,
            ao_safe_area_bottom: 0,
            ao_safe_area_left: 0,
            ao_safe_area_right: 0,
            kiosk: false,
            display_mode: String::new(),
            radio_mode: "fm".to_string(),
            fm_frequency: crate::radio::fm::DEFAULT_FREQUENCY_KHZ,
            fm_favorites: Vec::new(),
            dab_stations: Vec::new(),
            dab_favorites: Vec::new(),
            dab_last: None,
        }
    }
}

static SETTINGS: Mutex<Option<AppSettings>> = Mutex::new(None);
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static REMOTE: Remote = Remote::new();

fn settings_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(env!("CARGO_PKG_NAME")).join("settings.json"))
}

// Call once at startup, before Qt creates the singletons that read the settings
pub fn load() {
    let loaded = settings_path()
        .and_then(|path| match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                eprintln!("settings: reading {}: {e}", path.display());
                None
            }
        })
        .and_then(|text| {
            serde_json::from_str(&text)
                .inspect_err(|e| eprintln!("settings: ignoring malformed file: {e}"))
                .ok()
        })
        .unwrap_or_default();
    *SETTINGS.lock().expect("settings lock poisoned") = Some(loaded);
}

pub fn snapshot() -> AppSettings {
    SETTINGS
        .lock()
        .expect("settings lock poisoned")
        .get_or_insert_with(AppSettings::default)
        .clone()
}

// Changes the settings and schedules a save once things have been quiet for SAVE_DEBOUNCE_MS
pub fn update(change: impl FnOnce(&mut AppSettings)) {
    change(
        SETTINGS
            .lock()
            .expect("settings lock poisoned")
            .get_or_insert_with(AppSettings::default),
    );

    let generation = SAVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    crate::runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SAVE_DEBOUNCE_MS)).await;
        // A newer update superseded this one
        if SAVE_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Err(e) = save(snapshot()).await {
            eprintln!("settings: save failed: {e}");
        }
    });
}

async fn save(settings: AppSettings) -> Result<(), String> {
    let path = settings_path().ok_or("no config directory on this platform")?;
    let dir = path.parent().ok_or("settings path has no parent")?;
    tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;

    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    // Write beside the target and rename, so a crash can't leave a half-written file
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, text).await.map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, &path).await.map_err(|e| e.to_string())
}

// Called by the worker (commands.rs) to report back on the Qt thread
pub fn reply(call: impl FnOnce(&qtbridge::QmlMethodInvoker)) {
    REMOTE.with(call);
}

pub struct SettingsStore {
    master_volume: u8,
    default_sound_device: String,
    default_input_device: String,
    audio_devices: Vec<String>,
    input_devices: Vec<String>,
    brightness: u32,
    kiosk: bool,
    display_mode: String,
    display_modes: Vec<String>,
    ao_resolution: String,
    ao_framerate: String,
    ao_dpi: f64,
    ao_view_area_top: u32,
    ao_view_area_bottom: u32,
    ao_view_area_left: u32,
    ao_view_area_right: u32,
    ao_safe_area_top: u32,
    ao_safe_area_bottom: u32,
    ao_safe_area_left: u32,
    ao_safe_area_right: u32,
}

impl Default for SettingsStore {
    fn default() -> Self {
        let saved = snapshot();
        Self {
            master_volume: saved.master_volume,
            default_sound_device: saved.default_sound_device,
            default_input_device: saved.default_input_device,
            audio_devices: Vec::new(),
            input_devices: Vec::new(),
            brightness: saved.brightness,
            kiosk: saved.kiosk,
            display_mode: saved.display_mode,
            display_modes: crate::window::display_mode::list_host_output_modes(),
            ao_resolution: saved.ao_resolution,
            ao_framerate: saved.ao_framerate,
            ao_dpi: saved.ao_dpi,
            ao_view_area_top: saved.ao_view_area_top,
            ao_view_area_bottom: saved.ao_view_area_bottom,
            ao_view_area_left: saved.ao_view_area_left,
            ao_view_area_right: saved.ao_view_area_right,
            ao_safe_area_top: saved.ao_safe_area_top,
            ao_safe_area_bottom: saved.ao_safe_area_bottom,
            ao_safe_area_left: saved.ao_safe_area_left,
            ao_safe_area_right: saved.ao_safe_area_right,
        }
    }
}

// Writable properties apply the change to the system, then save. The *_loaded slots take values the
// worker read from the system, so they only update the property and never loop back into the system.
#[qobject(Singleton)]
impl SettingsStore {
    qproperty!("masterVolume", Read = master_volume, Write = set_master_volume, Notify = master_volume_changed);
    qproperty!("defaultSoundDevice", Read = default_sound_device, Write = set_default_sound_device, Notify = default_sound_device_changed);
    qproperty!("defaultInputDevice", Read = default_input_device, Write = set_default_input_device, Notify = default_input_device_changed);
    qproperty!("brightness", Read = brightness, Write = set_brightness, Notify = brightness_changed);
    qproperty!("audioDevices", Read = audio_devices, Notify = audio_devices_changed);
    qproperty!("inputDevices", Read = input_devices, Notify = input_devices_changed);
    qproperty!("kiosk", Read = kiosk, Write = set_kiosk, Notify = kiosk_changed);
    qproperty!("displayMode", Read = display_mode, Write = set_display_mode, Notify = display_mode_changed);
    qproperty!("displayModes", Read = display_modes, Notify = display_modes_changed);
    qproperty!("aoResolution", Read = ao_resolution, Write = set_ao_resolution, Notify = ao_resolution_changed);
    qproperty!("aoFramerate", Read = ao_framerate, Write = set_ao_framerate, Notify = ao_framerate_changed);
    qproperty!("aoDpi", Read = ao_dpi, Write = set_ao_dpi, Notify = ao_dpi_changed);
    qproperty!("aoViewAreaTop", Read = ao_view_area_top, Write = set_ao_view_area_top, Notify = ao_geometry_changed);
    qproperty!("aoViewAreaBottom", Read = ao_view_area_bottom, Write = set_ao_view_area_bottom, Notify = ao_geometry_changed);
    qproperty!("aoViewAreaLeft", Read = ao_view_area_left, Write = set_ao_view_area_left, Notify = ao_geometry_changed);
    qproperty!("aoViewAreaRight", Read = ao_view_area_right, Write = set_ao_view_area_right, Notify = ao_geometry_changed);
    qproperty!("aoSafeAreaTop", Read = ao_safe_area_top, Write = set_ao_safe_area_top, Notify = ao_geometry_changed);
    qproperty!("aoSafeAreaBottom", Read = ao_safe_area_bottom, Write = set_ao_safe_area_bottom, Notify = ao_geometry_changed);
    qproperty!("aoSafeAreaLeft", Read = ao_safe_area_left, Write = set_ao_safe_area_left, Notify = ao_geometry_changed);
    qproperty!("aoSafeAreaRight", Read = ao_safe_area_right, Write = set_ao_safe_area_right, Notify = ao_geometry_changed);

    #[qsignal]
    fn master_volume_changed(&mut self);
    #[qsignal]
    fn default_sound_device_changed(&mut self);
    #[qsignal]
    fn default_input_device_changed(&mut self);
    #[qsignal]
    fn brightness_changed(&mut self);
    #[qsignal]
    fn audio_devices_changed(&mut self);
    #[qsignal]
    fn input_devices_changed(&mut self);
    #[qsignal]
    fn kiosk_changed(&mut self);
    #[qsignal]
    fn display_mode_changed(&mut self);
    #[qsignal]
    fn display_modes_changed(&mut self);
    #[qsignal]
    fn ao_resolution_changed(&mut self);
    #[qsignal]
    fn ao_framerate_changed(&mut self);
    #[qsignal]
    fn ao_dpi_changed(&mut self);
    #[qsignal]
    fn ao_geometry_changed(&mut self);

    pub fn master_volume(&self) -> u8 {
        self.master_volume
    }

    pub fn default_sound_device(&self) -> String {
        self.default_sound_device.clone()
    }

    pub fn default_input_device(&self) -> String {
        self.default_input_device.clone()
    }

    pub fn brightness(&self) -> u32 {
        self.brightness
    }

    pub fn audio_devices(&self) -> Vec<String> {
        self.audio_devices.clone()
    }

    pub fn input_devices(&self) -> Vec<String> {
        self.input_devices.clone()
    }

    pub fn kiosk(&self) -> bool {
        self.kiosk
    }

    pub fn display_mode(&self) -> String {
        self.display_mode.clone()
    }

    pub fn display_modes(&self) -> Vec<String> {
        self.display_modes.clone()
    }

    pub fn ao_resolution(&self) -> String {
        self.ao_resolution.clone()
    }

    pub fn ao_framerate(&self) -> String {
        self.ao_framerate.clone()
    }

    pub fn ao_dpi(&self) -> f64 {
        self.ao_dpi
    }

    pub fn ao_view_area_top(&self) -> u32 {
        self.ao_view_area_top
    }

    pub fn ao_view_area_bottom(&self) -> u32 {
        self.ao_view_area_bottom
    }

    pub fn ao_view_area_left(&self) -> u32 {
        self.ao_view_area_left
    }

    pub fn ao_view_area_right(&self) -> u32 {
        self.ao_view_area_right
    }

    pub fn ao_safe_area_top(&self) -> u32 {
        self.ao_safe_area_top
    }

    pub fn ao_safe_area_bottom(&self) -> u32 {
        self.ao_safe_area_bottom
    }

    pub fn ao_safe_area_left(&self) -> u32 {
        self.ao_safe_area_left
    }

    pub fn ao_safe_area_right(&self) -> u32 {
        self.ao_safe_area_right
    }

    pub fn set_master_volume(&mut self, value: u8) {
        let value = value.min(100);
        self.master_volume = value;
        self.master_volume_changed();
        update(|s| s.master_volume = value);
        commands::send(Commands::SetVolume(value));
    }

    pub fn set_default_sound_device(&mut self, value: String) {
        self.default_sound_device = value.clone();
        self.default_sound_device_changed();
        update(|s| s.default_sound_device = value.clone());
        commands::send(Commands::SetDefaultSoundDevice(value));
    }

    pub fn set_default_input_device(&mut self, value: String) {
        self.default_input_device = value.clone();
        self.default_input_device_changed();
        update(|s| s.default_input_device = value.clone());
        commands::send(Commands::SetDefaultInputDevice(value));
    }

    pub fn set_brightness(&mut self, value: u32) {
        self.brightness = value;
        self.brightness_changed();
        update(|s| s.brightness = value);
        commands::send(Commands::SetBrightness(value));
    }

    pub fn set_kiosk(&mut self, value: bool) {
        self.kiosk = value;
        self.kiosk_changed();
        update(|s| s.kiosk = value);
    }

    pub fn set_display_mode(&mut self, value: String) {
        self.display_mode = value.clone();
        self.display_mode_changed();
        update(|s| s.display_mode = value.clone());
    }

    pub fn set_ao_resolution(&mut self, value: String) {
        self.ao_resolution = value.clone();
        self.ao_resolution_changed();
        update(|s| s.ao_resolution = value.clone());
    }

    pub fn set_ao_framerate(&mut self, value: String) {
        self.ao_framerate = value.clone();
        self.ao_framerate_changed();
        update(|s| s.ao_framerate = value.clone());
    }

    pub fn set_ao_dpi(&mut self, value: f64) {
        self.ao_dpi = value;
        self.ao_dpi_changed();
        update(|s| s.ao_dpi = value);
    }

    pub fn set_ao_view_area_top(&mut self, value: u32) {
        self.ao_view_area_top = value;
        self.ao_geometry_changed();
        update(|s| s.ao_view_area_top = value);
    }

    pub fn set_ao_view_area_bottom(&mut self, value: u32) {
        self.ao_view_area_bottom = value;
        self.ao_geometry_changed();
        update(|s| s.ao_view_area_bottom = value);
    }

    pub fn set_ao_view_area_left(&mut self, value: u32) {
        self.ao_view_area_left = value;
        self.ao_geometry_changed();
        update(|s| s.ao_view_area_left = value);
    }

    pub fn set_ao_view_area_right(&mut self, value: u32) {
        self.ao_view_area_right = value;
        self.ao_geometry_changed();
        update(|s| s.ao_view_area_right = value);
    }

    pub fn set_ao_safe_area_top(&mut self, value: u32) {
        self.ao_safe_area_top = value;
        self.ao_geometry_changed();
        update(|s| s.ao_safe_area_top = value);
    }

    pub fn set_ao_safe_area_bottom(&mut self, value: u32) {
        self.ao_safe_area_bottom = value;
        self.ao_geometry_changed();
        update(|s| s.ao_safe_area_bottom = value);
    }

    pub fn set_ao_safe_area_left(&mut self, value: u32) {
        self.ao_safe_area_left = value;
        self.ao_geometry_changed();
        update(|s| s.ao_safe_area_left = value);
    }

    pub fn set_ao_safe_area_right(&mut self, value: u32) {
        self.ao_safe_area_right = value;
        self.ao_geometry_changed();
        update(|s| s.ao_safe_area_right = value);
    }

    // Call once from QML on startup: registers for replies and reads the current system state
    #[qslot]
    fn init(&self) {
        REMOTE.set(self.get_qml_method_invoker());
        commands::send(Commands::RefreshAll);
    }

    #[qslot]
    fn refresh_devices(&self) {
        commands::send(Commands::RefreshDevices);
    }

    #[qslot]
    fn volume_loaded(&mut self, value: u8) {
        self.master_volume = value;
        self.master_volume_changed();
    }

    #[qslot]
    fn brightness_loaded(&mut self, value: u32) {
        self.brightness = value;
        self.brightness_changed();
    }

    #[qslot]
    fn default_sound_device_loaded(&mut self, value: String) {
        self.default_sound_device = value;
        self.default_sound_device_changed();
    }

    #[qslot]
    fn default_input_device_loaded(&mut self, value: String) {
        self.default_input_device = value;
        self.default_input_device_changed();
    }

    #[qslot]
    fn audio_devices_loaded(&mut self, value: Vec<String>) {
        self.audio_devices = value;
        self.audio_devices_changed();
    }

    #[qslot]
    fn input_devices_loaded(&mut self, value: Vec<String>) {
        self.input_devices = value;
        self.input_devices_changed();
    }
}
