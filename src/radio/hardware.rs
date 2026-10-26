use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DabBackendKind {
    WelleIo,
    WradioC100,
}

impl DabBackendKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::WelleIo => "WelleIo",
            Self::WradioC100 => "WradioC100",
        }
    }
}

/// What the plugged-in USB hardware can do. `fm` is RTL-SDR only; `dab` names the backend that
/// would serve DAB (an RTL-SDR can do either mode, the WRadio C100 only DAB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RadioHardware {
    pub fm: bool,
    pub dab: Option<DabBackendKind>,
}

impl RadioHardware {
    pub fn detect(wradio_present: bool, rtl_sdr_present: bool) -> Self {
        let dab = if wradio_present {
            Some(DabBackendKind::WradioC100)
        } else if rtl_sdr_present {
            Some(DabBackendKind::WelleIo)
        } else {
            None
        };

        Self {
            fm: rtl_sdr_present,
            dab,
        }
    }
}

static HARDWARE: Mutex<RadioHardware> = Mutex::new(RadioHardware { fm: false, dab: None });

pub fn get() -> RadioHardware {
    *HARDWARE.lock().expect("radio hardware lock poisoned")
}

pub fn set(hardware: RadioHardware) {
    *HARDWARE.lock().expect("radio hardware lock poisoned") = hardware;
}
