use serde::{Deserialize, Serialize};

use super::Notifier;
use super::hardware::{DabBackendKind, RadioHardware};
use super::wradio_backend::WradioBackend;

pub const DAB_FAVORITE_SLOTS: usize = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DabStation {
    pub id: u32,
    pub label: String,
    pub channel: u32,
    pub frequency: u32,
}

/// What a scan is doing right now. A channel is reported when the scan starts looking at it
/// (`locked` and `failed` both false), then again with its result.
#[derive(Debug, Clone)]
pub struct DabScanProgress {
    pub channel_index: usize,
    pub channel_count: usize,
    pub frequency_khz: u32,
    /// welle.io's name for the channel (e.g. "12B").
    pub channel: Option<&'static str>,
    pub locked: bool,
    /// No signal on this channel, as opposed to one that's still being looked at.
    pub failed: bool,
    /// The ensemble found on this channel, once its label is known.
    pub ensemble: Option<String>,
    pub service_count: usize,
}

impl DabScanProgress {
    pub fn describe(&self) -> String {
        let position = format!("{}/{}", self.channel_index + 1, self.channel_count);
        let mhz = format!("{:.3} MHz", f64::from(self.frequency_khz) / 1000.0);
        let frequency = match self.channel {
            Some(channel) => format!("{channel} ({mhz})"),
            None => mhz,
        };
        match (&self.ensemble, self.failed, self.locked) {
            (_, true, _) => format!("No signal on {frequency} ({position})"),
            (Some(ensemble), _, true) => {
                format!("Found {ensemble} · {} services ({position})", self.service_count)
            }
            (None, _, true) => format!("No ensemble on {frequency} ({position})"),
            _ => format!("Scanning {frequency} ({position})"),
        }
    }
}

/// The DAB implementation driving the plugged-in hardware. The welle.io (RTL-SDR) one doesn't
/// exist yet, so `open` is the only place a backend kind can be refused.
pub enum DabBackend {
    Wradio(WradioBackend),
}

impl DabBackend {
    fn create(kind: DabBackendKind, notify: Notifier) -> Result<Self, String> {
        match kind {
            DabBackendKind::WelleIo => Err("The welle.io DAB backend is not implemented yet".into()),
            DabBackendKind::WradioC100 => Ok(Self::Wradio(WradioBackend::new(notify))),
        }
    }

    async fn open(&mut self) -> Result<(), String> {
        match self {
            Self::Wradio(backend) => backend.open().await,
        }
    }

    async fn close(&mut self) -> Result<(), String> {
        match self {
            Self::Wradio(backend) => backend.close().await,
        }
    }

    async fn scan(
        &mut self,
        progress: &mut (dyn FnMut(DabScanProgress) + Send),
    ) -> Result<Vec<DabStation>, String> {
        match self {
            Self::Wradio(backend) => backend.scan(progress).await,
        }
    }

    async fn tune(&mut self, station: &DabStation) -> Result<(), String> {
        match self {
            Self::Wradio(backend) => backend.tune(station).await,
        }
    }

    async fn stop_playback(&mut self) -> Result<(), String> {
        match self {
            Self::Wradio(backend) => backend.stop_playback().await,
        }
    }

    fn dynamic_label(&self) -> Option<String> {
        match self {
            Self::Wradio(backend) => backend.dynamic_label(),
        }
    }
}

pub struct DabTuner {
    backend: Option<DabBackend>,
    playing: bool,
    stations: Vec<DabStation>,
    current: Option<DabStation>,
    favorites: Vec<Option<DabStation>>,
    notify: Notifier,
}

impl DabTuner {
    pub fn new(
        notify: Notifier,
        stations: Vec<DabStation>,
        favorites: Vec<Option<DabStation>>,
        current: Option<DabStation>,
    ) -> Self {
        let mut slots = vec![None; DAB_FAVORITE_SLOTS];
        for (slot, favorite) in slots.iter_mut().zip(favorites) {
            *slot = favorite;
        }
        Self {
            backend: None,
            playing: false,
            stations,
            current,
            favorites: slots,
            notify,
        }
    }

    pub fn running(&self) -> bool {
        self.backend.is_some()
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn stations(&self) -> &[DabStation] {
        &self.stations
    }

    pub fn current(&self) -> Option<&DabStation> {
        self.current.as_ref()
    }

    pub fn favorites(&self) -> &[Option<DabStation>] {
        &self.favorites
    }

    pub fn dynamic_label(&self) -> Option<String> {
        self.backend.as_ref().and_then(DabBackend::dynamic_label)
    }

    pub fn find(&self, id: u32, frequency: u32) -> Option<DabStation> {
        self.stations
            .iter()
            .find(|s| s.id == id && s.frequency == frequency)
            .cloned()
    }

    pub async fn start(&mut self, hardware: RadioHardware) -> Result<(), String> {
        if self.backend.is_some() {
            return Ok(());
        }
        let kind = hardware.dab.ok_or("No DAB-capable tuner is plugged in")?;
        let mut backend = DabBackend::create(kind, self.notify.clone())?;
        backend.open().await?;
        self.backend = Some(backend);
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), String> {
        self.playing = false;
        match self.backend.take() {
            Some(mut backend) => backend.close().await,
            None => Ok(()),
        }
    }

    /// Stops playing but leaves the tuner open; `current` is kept so it can be resumed.
    pub async fn stop_playback(&mut self) -> Result<(), String> {
        if let Some(backend) = self.backend.as_mut() {
            backend.stop_playback().await?;
        }
        self.playing = false;
        Ok(())
    }

    /// Replaces the station list with what the scan found. An empty result (bad antenna, tuner
    /// hiccup) keeps the previous list instead of wiping it.
    pub async fn scan(
        &mut self,
        progress: &mut (dyn FnMut(DabScanProgress) + Send),
    ) -> Result<(), String> {
        let backend = self.backend.as_mut().ok_or("DAB is not running")?;
        // Scanning retunes the hardware, so whatever is playing has to stop first
        if self.playing {
            backend.stop_playback().await?;
            self.playing = false;
        }
        let stations = backend.scan(progress).await?;
        if !stations.is_empty() {
            self.stations = stations;
        }
        Ok(())
    }

    pub async fn tune(&mut self, station: DabStation) -> Result<(), String> {
        let backend = self.backend.as_mut().ok_or("DAB is not running")?;
        backend.tune(&station).await?;
        self.current = Some(station);
        self.playing = true;
        Ok(())
    }

    pub fn set_favorite(&mut self, slot: usize) -> Result<(), String> {
        let favorite = self
            .favorites
            .get_mut(slot)
            .ok_or_else(|| format!("Favorite slot {slot} out of range"))?;
        *favorite = self.current.clone();
        Ok(())
    }

    pub async fn recall_favorite(&mut self, slot: usize) -> Result<(), String> {
        let station = self
            .favorites
            .get(slot)
            .cloned()
            .flatten()
            .ok_or_else(|| format!("Favorite slot {slot} is empty"))?;
        self.tune(station).await
    }
}
