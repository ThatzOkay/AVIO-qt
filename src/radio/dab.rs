use std::path::Path;

use serde::{Deserialize, Serialize};

use super::Notifier;
use super::hardware::{DabBackendKind, RadioHardware};
use super::wradio_backend::{TuneJob, WradioBackend};

pub const DAB_FAVORITE_SLOTS: usize = 5;

const STATION_LIST_VERSION: u32 = 1;

/// The station list as exported to / imported from a file, to move it between devices (e.g. scan
/// once on a desktop, import on the head unit).
#[derive(Serialize, Deserialize)]
struct StationList {
    version: u32,
    stations: Vec<DabStation>,
    favorites: Vec<Option<DabStation>>,
}

/// The file name an export gets inside the chosen folder.
const STATION_LIST_FILE: &str = "avio-dab-stations.json";

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

    fn tune_job(&self) -> Result<TuneJob, String> {
        match self {
            Self::Wradio(backend) => backend.tune_job(),
        }
    }

    fn tune_status(&self) -> Option<String> {
        match self {
            Self::Wradio(backend) => backend.tune_status(),
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

    fn signal_bars(&self) -> Option<u8> {
        match self {
            Self::Wradio(backend) => backend.signal_bars(),
        }
    }

    fn audio_ready(&self) -> bool {
        match self {
            Self::Wradio(backend) => backend.audio_ready(),
        }
    }

    fn audio_error(&self) -> Option<String> {
        match self {
            Self::Wradio(backend) => backend.audio_error(),
        }
    }
}

pub struct DabTuner {
    backend: Option<DabBackend>,
    playing: bool,
    // The station a tune is retrying for, until it plays or is cancelled
    tuning: Option<DabStation>,
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
            tuning: None,
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

    pub fn tuning(&self) -> Option<&DabStation> {
        self.tuning.as_ref()
    }

    /// Why the running tune is still retrying, e.g. "No signal on 12B".
    pub fn tune_status(&self) -> Option<String> {
        self.backend.as_ref().and_then(DabBackend::tune_status)
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

    /// Whether decoded audio has started reaching the output since the last tune. `playing` is
    /// true as soon as the tune was accepted, before this.
    pub fn audio_ready(&self) -> bool {
        self.backend.as_ref().is_some_and(DabBackend::audio_ready)
    }

    /// Signal bars (0..=6) while a station is playing; the reading is stale otherwise.
    pub fn signal_bars(&self) -> Option<u8> {
        if !self.playing {
            return None;
        }
        self.backend.as_ref().and_then(DabBackend::signal_bars)
    }

    pub fn audio_error(&self) -> Option<String> {
        self.backend.as_ref().and_then(DabBackend::audio_error)
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
        self.tuning = None;
        match self.backend.take() {
            Some(mut backend) => backend.close().await,
            None => Ok(()),
        }
    }

    /// Stops playing but leaves the tuner open; `current` is kept so it can be resumed.
    pub async fn stop_playback(&mut self) -> Result<(), String> {
        self.playing = false;
        self.tuning = None;
        if let Some(backend) = self.backend.as_mut() {
            backend.stop_playback().await?;
        }
        Ok(())
    }

    /// Replaces the station list with what the scan found. An empty result (bad antenna, tuner
    /// hiccup) keeps the previous list instead of wiping it.
    pub async fn scan(
        &mut self,
        progress: &mut (dyn FnMut(DabScanProgress) + Send),
    ) -> Result<(), String> {
        let backend = self.backend.as_mut().ok_or("DAB is not running")?;
        // Scanning retunes the hardware, so whatever is playing (or being tuned) has to stop first
        if self.playing || self.tuning.is_some() {
            backend.stop_playback().await?;
            self.playing = false;
            self.tuning = None;
        }
        let stations = backend.scan(progress).await?;
        if !stations.is_empty() {
            self.stations = stations;
        }
        Ok(())
    }

    /// Starts a tune. The returned job does the work, off `self`, so the caller can keep serving
    /// commands while it retries and drop it to cancel; report how it went with `finish_tune`.
    pub fn begin_tune(&mut self, station: DabStation) -> Result<TuneJob, String> {
        let job = self.backend.as_ref().ok_or("DAB is not running")?.tune_job()?;
        self.playing = false;
        self.tuning = Some(station);
        Ok(job)
    }

    pub fn finish_tune(&mut self, station: DabStation, result: &Result<(), String>) {
        if result.is_ok() {
            self.current = Some(station);
            self.playing = true;
        }
        self.tuning = None;
    }

    /// Writes the stations and favorites to `STATION_LIST_FILE` in `folder`. Returns a message
    /// for the UI.
    pub async fn export_list(&self, folder: &Path) -> Result<String, String> {
        let path = folder.join(STATION_LIST_FILE);
        let list = StationList {
            version: STATION_LIST_VERSION,
            stations: self.stations.clone(),
            favorites: self.favorites.clone(),
        };
        let text = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
        tokio::fs::write(&path, text)
            .await
            .map_err(|e| format!("Writing {}: {e}", path.display()))?;
        Ok(format!("Exported {} stations to {}", self.stations.len(), path.display()))
    }

    /// Replaces the stations and favorites with those in the file at `path`. A file with no
    /// usable stations is refused rather than wiping the current list. Returns a message for the UI.
    pub async fn import_list(&mut self, path: &Path) -> Result<String, String> {
        let text = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| format!("Reading {}: {e}", path.display()))?;
        let list: StationList =
            serde_json::from_str(&text).map_err(|e| format!("{} is not a station list: {e}", path.display()))?;
        if list.version != STATION_LIST_VERSION {
            return Err(format!("Unsupported station list version {}", list.version));
        }

        let stations: Vec<_> = list.stations.into_iter().filter(|s| !s.label.is_empty()).collect();
        if stations.is_empty() {
            return Err(format!("{} has no stations", path.display()));
        }
        let mut favorites = vec![None; DAB_FAVORITE_SLOTS];
        for (slot, favorite) in favorites.iter_mut().zip(list.favorites) {
            *slot = favorite;
        }

        let count = stations.len();
        self.stations = stations;
        self.favorites = favorites;
        Ok(format!("Imported {count} stations from {}", path.display()))
    }

    pub fn set_favorite(&mut self, slot: usize) -> Result<(), String> {
        let favorite = self
            .favorites
            .get_mut(slot)
            .ok_or_else(|| format!("Favorite slot {slot} out of range"))?;
        // What's being tuned counts: holding a preset while a station is still locking saves that one
        *favorite = self.tuning.clone().or_else(|| self.current.clone());
        Ok(())
    }

    pub fn favorite(&self, slot: usize) -> Result<DabStation, String> {
        self.favorites
            .get(slot)
            .cloned()
            .flatten()
            .ok_or_else(|| format!("Favorite slot {slot} is empty"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(id: u32, label: &str, frequency: u32) -> DabStation {
        DabStation { id, label: label.to_string(), channel: 0, frequency }
    }

    fn tuner(stations: Vec<DabStation>, favorites: Vec<Option<DabStation>>) -> DabTuner {
        let (notify, _) = tokio::sync::mpsc::unbounded_channel();
        DabTuner::new(notify, stations, favorites, None)
    }

    #[tokio::test]
    async fn station_list_round_trips() {
        let dir = std::env::temp_dir().join(format!("avio-dab-list-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(STATION_LIST_FILE);

        let a = station(1, "Radio 1", 225_648);
        let b = station(2, "Radio 2", 225_648);
        let exported = tuner(vec![a.clone(), b.clone()], vec![None, Some(b.clone())]);
        assert!(exported.export_list(&dir).await.unwrap().contains("2 stations"));

        let mut imported = tuner(vec![station(9, "Old", 1)], Vec::new());
        imported.import_list(&file).await.unwrap();
        assert_eq!(imported.stations(), [a, b.clone()]);
        assert_eq!(imported.favorites().len(), DAB_FAVORITE_SLOTS);
        assert_eq!(imported.favorites()[1], Some(b));

        // A list with nothing usable must not wipe what's there
        std::fs::write(&file, r#"{"version":1,"stations":[],"favorites":[]}"#).unwrap();
        assert!(imported.import_list(&file).await.is_err());
        assert_eq!(imported.stations().len(), 2);

        // Nor a file from some other version
        std::fs::write(&file, r#"{"version":2,"stations":[],"favorites":[]}"#).unwrap();
        assert!(imported.import_list(&file).await.unwrap_err().contains("version"));

        // Nor something that isn't a station list at all
        std::fs::write(&file, "nope").unwrap();
        assert!(imported.import_list(&file).await.is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
