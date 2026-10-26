use std::sync::OnceLock;

#[cfg(not(target_os = "linux"))]
use cpvc::get_system_volume;
use regex::Regex;

#[cfg(target_os = "linux")]
use libpulse_binding as pulse;
#[cfg(target_os = "linux")]
use pulse::callbacks::ListResult;
#[cfg(target_os = "linux")]
use pulse::context::{Context, FlagSet};
#[cfg(target_os = "linux")]
use pulse::mainloop::standard::Mainloop;

use crate::audio::audio_device_enumerator::{list_audio_devices, AudioDevice, AudioDeviceType};

pub mod audio_device_enumerator;
pub mod audio_downsample;
pub mod audio_output;
pub mod gstreamer;
// Capture-side mic support, not yet wired to the AA MicChannel path.
#[allow(dead_code)]
pub mod microphone;

struct DedupedPerMac {
    deduped: Vec<AudioDevice>,
    #[allow(dead_code)]
    macs: std::collections::HashSet<String>,
}

// Bluetooth-audio dedup helpers, not yet wired into device enumeration.
#[allow(dead_code)]
const BT_COD_MAJOR_AUDIO: u8 = 0x04;
static BLUEZ_MAC_RE: OnceLock<Regex> = OnceLock::new();

#[allow(dead_code)]
fn is_bt_audio_cod(cod: u32) -> bool {
    let major_cod = ((cod >> 8) & 0x1F) as u8;
    major_cod == BT_COD_MAJOR_AUDIO
}

#[allow(dead_code)]
fn mac_to_bluez_id(mac: &str) -> Option<String> {
    mac.to_uppercase().replace(":", "_").into()
}

fn extract_bluez_mac(device_id: &str) -> Option<String> {
    let re = BLUEZ_MAC_RE.get_or_init(|| {
        Regex::new(r"^bluez_(?:output|input|sink|source)\.([0-9A-Fa-f_:]{17})").unwrap()
    });
    re.captures(device_id)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().replace('_', ":").to_uppercase())
}

fn dedupe_bt_per_mac(devices: Vec<AudioDevice>) -> DedupedPerMac {
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();
    for device in devices {
        let mac = extract_bluez_mac(&device.id);
        if let Some(mac) = mac {
            if seen.contains(&mac) {
                continue;
            }
            seen.insert(mac);
        }
        deduped.push(device);
    }
    DedupedPerMac {
        deduped,
        macs: seen,
    }
}

async fn mixed_audio_devices(kind: AudioDeviceType) -> Result<Vec<AudioDevice>, String> {
    let local = list_audio_devices(kind).await?;
    let deduped = dedupe_bt_per_mac(local);
    Ok(deduped.deduped)
}

pub async fn list_sinks() -> Result<Vec<AudioDevice>, String> {
    mixed_audio_devices(AudioDeviceType::Sink).await
}

pub async fn list_sources() -> Result<Vec<AudioDevice>, String> {
    mixed_audio_devices(AudioDeviceType::Source).await
}

// cpvc's PulseAudio backend picks "the last sink that has an active port" rather than the real
// default sink (so it can act on a virtual sink like Easy Effects) and opens a new server connection
// per call. pactl with @DEFAULT_SINK@ targets the actual default and is quick.
#[cfg(target_os = "linux")]
fn pactl(args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("pactl")
        .args(args)
        .output()
        .map_err(|e| format!("pactl: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

// 0 while muted, matching cpvc and how the UI treats 0
pub fn get_current_volume() -> Result<u8, String> {
    #[cfg(target_os = "linux")]
    {
        if get_mute()? {
            return Ok(0);
        }
        let output = pactl(&["get-sink-volume", "@DEFAULT_SINK@"])?;
        let percent = Regex::new(r"(\d+)%")
            .map_err(|e| e.to_string())?
            .captures(&output)
            .and_then(|c| c[1].parse::<u32>().ok())
            .ok_or_else(|| format!("unexpected pactl output: {output}"))?;
        Ok(percent.min(u32::from(u8::MAX)) as u8)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(get_system_volume())
    }
}

pub async fn get_audio_devices() -> Result<Vec<String>, String> {
    let devices: Vec<String> = cpvc::get_sound_devices();
    Ok(devices)
}

// Sources (microphones) have their own name/description pairs, distinct from
// sinks (e.g. "alsa_input.*" vs "alsa_output.*") — cpvc's get_device_id only
// knows about sinks, so source name<->description lookups are done here
// directly against libpulse instead. PulseAudio is Linux-only (see Cargo.toml)
// - not yet implemented on Windows/macOS (would need WASAPI/CoreAudio, like
// `set_default_device_windows`/`set_default_device_macos` below already do for sinks).
#[cfg(target_os = "linux")]
fn get_source_identifiers() -> Result<Vec<(String, String)>, String> {
    let identifiers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let identifiers_clone = identifiers.clone();

    let mut mainloop = Mainloop::new().ok_or("Failed to create PulseAudio mainloop")?;

    let mut context =
        Context::new(&mainloop, "tauri-audio").ok_or("Failed to create PulseAudio context")?;

    context
        .connect(None, FlagSet::NOFLAGS, None)
        .map_err(|e| format!("Failed to connect: {:?}", e))?;

    while context.get_state() != pulse::context::State::Ready {
        mainloop.iterate(true);
    }

    let mainloop_api = mainloop.get_api();

    context
        .introspect()
        .get_source_info_list(move |result| match result {
            ListResult::Item(source) => {
                if let (Some(name), Some(description)) = (&source.name, &source.description) {
                    let is_monitor = name.ends_with(".monitor");

                    if !is_monitor {
                        identifiers_clone
                            .lock()
                            .unwrap()
                            .push((name.to_string(), description.to_string()));
                    }
                }
            }

            ListResult::End => {
                if let Some(quit) = mainloop_api.quit {
                    quit(mainloop_api, 0);
                }
            }

            ListResult::Error => {
                if let Some(quit) = mainloop_api.quit {
                    quit(mainloop_api, 1);
                }
            }
        });

    while mainloop.iterate(true).is_success() {}

    let identifiers = identifiers.lock().unwrap().clone();
    Ok(identifiers)
}

#[cfg(not(target_os = "linux"))]
fn get_source_identifiers() -> Result<Vec<(String, String)>, String> {
    Ok(Vec::new())
}

pub fn get_input_devices() -> Result<Vec<String>, String> {
    Ok(get_source_identifiers()?
        .into_iter()
        .map(|(_name, description)| description)
        .collect())
}

pub fn get_default_input_device_name() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("pactl")
            .args(["get-default-source"])
            .output()
            .map_err(|e| format!("Failed to execute pactl: {:?}", e))?;

        if !output.status.success() {
            return Err(format!(
                "pactl returned non-zero exit status: {}",
                output.status
            ));
        }

        let source_name = String::from_utf8_lossy(&output.stdout).trim().to_string();

        get_source_identifiers()?
            .into_iter()
            .find(|(name, _description)| *name == source_name)
            .map(|(_name, description)| description)
            .ok_or_else(|| format!("Default source '{source_name}' not found in source list"))
    }

    #[cfg(target_os = "windows")]
    {
        Err("Getting the default input device is not supported on Windows".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        Err("Getting the default input device is not supported on macOS".to_string())
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Err("Getting the default input device is not supported on this platform".to_string())
    }
}

pub fn set_default_input_device(device_name: &str) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        let source_name = get_source_identifiers()?
            .into_iter()
            .find(|(_name, description)| description == device_name)
            .map(|(name, _description)| name)
            .ok_or_else(|| format!("Device not found: {device_name}"))?;

        let status = std::process::Command::new("pactl")
            .args(["set-default-source", &source_name])
            .status()
            .map_err(|e| e.to_string())?;

        Ok(status.success())
    }

    #[cfg(target_os = "windows")]
    {
        Err("Setting the default input device is not supported on Windows".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        Err("Setting the default input device is not supported on macOS".to_string())
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = device_name;
        Err("Setting the default input device is not supported on this platform".to_string())
    }
}

// The Windows Core Audio API has no documented way to change the default
// endpoint; every shipping tool that does it (NirCmd, AudioDeviceCmdlets,
// svcl, ...) goes through this undocumented IPolicyConfig COM interface,
// which is why it isn't part of the `windows` crate's generated bindings and
// has to be declared by hand here. Layout is the widely used Windows
// 7-through-11 vtable (distinct from the older "Vista" variant).
#[cfg(target_os = "windows")]
mod policy_config {
    // Method names must match the COM interface exactly.
    #![allow(non_snake_case)]

    use windows::core::{interface, GUID, HRESULT, PCWSTR};
    use windows::Win32::Media::Audio::ERole;

    pub const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_values(
        0x870a_f99c,
        0x171d,
        0x4f9e,
        [0xaf, 0x0d, 0xe6, 0x3d, 0xf4, 0x0c, 0x2b, 0xc9],
    );

    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: windows::core::IUnknown {
        pub fn GetMixFormat(
            &self,
            device_id: PCWSTR,
            format: *mut *mut core::ffi::c_void,
        ) -> HRESULT;
        pub fn GetDeviceFormat(
            &self,
            device_id: PCWSTR,
            default: i32,
            format: *mut *mut core::ffi::c_void,
        ) -> HRESULT;
        pub fn ResetDeviceFormat(&self, device_id: PCWSTR) -> HRESULT;
        pub fn SetDeviceFormat(
            &self,
            device_id: PCWSTR,
            format: *mut core::ffi::c_void,
            mix_format: *mut core::ffi::c_void,
        ) -> HRESULT;
        pub fn GetProcessingPeriod(
            &self,
            device_id: PCWSTR,
            default: i32,
            default_period: *mut i64,
            minimum_period: *mut i64,
        ) -> HRESULT;
        pub fn SetProcessingPeriod(&self, device_id: PCWSTR, period: *mut i64) -> HRESULT;
        pub fn GetShareMode(&self, device_id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
        pub fn SetShareMode(&self, device_id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
        pub fn GetPropertyValue(
            &self,
            device_id: PCWSTR,
            key: *const core::ffi::c_void,
            value: *mut core::ffi::c_void,
        ) -> HRESULT;
        pub fn SetPropertyValue(
            &self,
            device_id: PCWSTR,
            key: *const core::ffi::c_void,
            value: *const core::ffi::c_void,
        ) -> HRESULT;
        pub fn SetDefaultEndpoint(
            &self,
            device_id: PCWSTR,
            role: ERole,
        ) -> windows::core::Result<()>;
        pub fn SetEndpointVisibility(&self, device_id: PCWSTR, visible: i32) -> HRESULT;
    }
}

#[cfg(target_os = "windows")]
fn set_default_device_windows(device_name: &str) -> Result<bool, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{eCommunications, eConsole, eMultimedia};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    let (uid, _name) = cpvc::wasapi::wasapi::get_device_identifiers()
        .map_err(|e| format!("{e:?}"))?
        .into_iter()
        .find(|(_, name)| name == device_name)
        .ok_or_else(|| format!("Device not found: {device_name}"))?;

    let mut wide_uid: Vec<u16> = uid.encode_utf16().chain(std::iter::once(0)).collect();
    let device_id = PCWSTR(wide_uid.as_mut_ptr());

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let policy_config: policy_config::IPolicyConfig =
            CoCreateInstance(&policy_config::CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)
                .map_err(|e| e.to_string())?;

        for role in [eConsole, eMultimedia, eCommunications] {
            policy_config
                .SetDefaultEndpoint(device_id, role)
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(true)
}

// cpvc only exposes CoreAudio device enumeration internally (`pub(super)`),
// so the device lookup here is re-implemented directly against
// objc2-core-audio; `get_device_name` below is the one piece cpvc does
// expose publicly, which saves re-deriving the CFString -> String dance.
#[cfg(target_os = "macos")]
fn set_default_device_macos(device_name: &str) -> Result<bool, String> {
    use core::ffi::c_void;
    use objc2_core_audio::{
        kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
        AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectSetPropertyData,
    };
    use std::ptr::{null, NonNull};

    let devices_address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDevices,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };

    let mut data_size: u32 = 0;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::new_unchecked(&devices_address as *const _ as *mut _),
            0,
            null(),
            NonNull::new_unchecked(&mut data_size),
        )
    };
    if status != 0 {
        return Err(format!("Failed to get audio device count: status {status}"));
    }

    let count = data_size as usize / std::mem::size_of::<AudioObjectID>();
    let mut device_ids: Vec<AudioObjectID> = vec![0; count];
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::new_unchecked(&devices_address as *const _ as *mut _),
            0,
            null(),
            NonNull::new_unchecked(&mut data_size),
            NonNull::new_unchecked(device_ids.as_mut_ptr() as *mut c_void),
        )
    };
    if status != 0 {
        return Err(format!(
            "Failed to enumerate audio devices: status {status}"
        ));
    }

    let device_id = device_ids
        .into_iter()
        .find(|id| {
            *id != 0
                && cpvc::coreaudio::coreaudio::get_device_name(*id)
                    .map(|name| name == device_name)
                    .unwrap_or(false)
        })
        .ok_or_else(|| format!("Device not found: {device_name}"))?;

    let default_address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDefaultOutputDevice,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };

    let status = unsafe {
        AudioObjectSetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::new_unchecked(&default_address as *const _ as *mut _),
            0,
            null(),
            std::mem::size_of::<AudioObjectID>() as u32,
            NonNull::new_unchecked(&device_id as *const _ as *mut c_void),
        )
    };

    Ok(status == 0)
}

pub async fn set_default_device(device_name: String) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        let sink_name = cpvc::pulseaudio::pulseaudio::get_device_id(device_name)
            .map_err(|e| format!("{e:?}"))?;

        let status = tokio::process::Command::new("pactl")
            .args(["set-default-sink", &sink_name])
            .status()
            .await
            .map_err(|e| e.to_string())?;

        Ok(status.success())
    }

    #[cfg(target_os = "windows")]
    {
        set_default_device_windows(&device_name)
    }

    #[cfg(target_os = "macos")]
    {
        set_default_device_macos(&device_name)
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = device_name;
        Err("Setting the default audio device is not supported on this platform".to_string())
    }
}

pub fn get_default_device_name() -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        cpvc::wasapi::wasapi::get_sound_devices()
            .map_err(|e| format!("{e:?}"))?
            .into_iter()
            .next()
            .ok_or_else(|| "No default device found".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        cpvc::coreaudio::coreaudio::get_sound_devices()
            .map_err(|e| format!("{e:?}"))?
            .into_iter()
            .next()
            .ok_or_else(|| "No default device found".to_string())
    }

    #[cfg(target_os = "linux")]
    {
        cpvc::pulseaudio::pulseaudio::get_sound_devices()
            .map_err(|e| format!("{e:?}"))?
            .into_iter()
            .next()
            .ok_or_else(|| "No default device found".to_string())
    }
}

pub fn set_current_volume(volume: u8) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        pactl(&["set-sink-volume", "@DEFAULT_SINK@", &format!("{volume}%")]).map(|_| true)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(cpvc::set_system_volume(volume))
    }
}

pub fn set_mute(mute: bool) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        pactl(&["set-sink-mute", "@DEFAULT_SINK@", if mute { "1" } else { "0" }]).map(|_| true)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(cpvc::set_mute(mute))
    }
}

pub fn get_mute() -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        Ok(pactl(&["get-sink-mute", "@DEFAULT_SINK@"])?.contains("yes"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(cpvc::get_mute())
    }
}
