use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;
use wradio_c100_core::adts;
use wradio_c100_core::device::{self, AudioFrame, Command, DeviceHandle, EnsembleInfo};
use wradio_c100_core::Tuner;

use crate::audio::audio_output::AudioOutput;
use super::Notifier;
use super::channels::channel_name;
use super::dab::{DabScanProgress, DabStation, merge_duplicates};
use super::slide_cache;

// A full Band III sweep waits on FIC collection per channel, so it takes a while on a weak
// antenna; this only guards against a scan that never reports finishing.
const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const SCAN_START_TIMEOUT: Duration = Duration::from_secs(5);
// After a tune the ensemble starts empty and fills in as the FIC is polled, which takes a few
// seconds; a service can only be started once it's listed.
const SERVICE_LIST_TIMEOUT: Duration = Duration::from_secs(15);
// Failed tunes on one frequency before a station with alternatives moves on to the next
const TRIES_PER_FREQUENCY: u32 = 2;
const AUDIO_REPORT_INTERVAL: Duration = Duration::from_secs(5);

/// Whether decoded audio is actually reaching the output, as opposed to a tune having been asked for.
#[derive(Default)]
struct AudioState {
    /// Frames have been written into an output that is still alive.
    ready: AtomicBool,
    /// Set when the output process died, e.g. its pipeline failed.
    error: Mutex<Option<String>>,
    /// When the current station started, until its first audio frame shows up (for the log)
    waiting_since: Mutex<Option<std::time::Instant>>,
    /// Tunes in progress. While there is one, frames still arriving from the previous station are
    /// dropped instead of being played over the new one.
    tuning: AtomicUsize,
}

/// Holds `AudioState::tuning` up for as long as a tune runs, however it ends (done, failed or
/// cancelled by dropping it).
struct TuneMute(Arc<AudioState>);

impl TuneMute {
    fn new(state: &Arc<AudioState>) -> Self {
        state.tuning.fetch_add(1, Ordering::SeqCst);
        Self(state.clone())
    }
}

impl Drop for TuneMute {
    fn drop(&mut self) {
        self.0.tuning.fetch_sub(1, Ordering::SeqCst);
    }
}

impl AudioState {
    fn reset(&self) {
        self.ready.store(false, Ordering::SeqCst);
        *self.error.lock().unwrap() = None;
        *self.waiting_since.lock().unwrap() = None;
    }
}

/// Drives the WRadio C100 (a generic Android USB DAB+ stick) through `wradio-c100-core`. Audio
/// leaves the core as raw DAB+ access units; they are ADTS-framed here and piped into GStreamer.
pub struct WradioBackend {
    notify: Notifier,
    device: Option<DeviceHandle>,
    audio: Option<Arc<tokio::sync::Mutex<AudioOutput>>>,
    dynamic_label: Arc<Mutex<Option<String>>>,
    // The newest slideshow picture of the tuned station (see slide_cache)
    slide: Arc<Mutex<Option<PathBuf>>>,
    audio_state: Arc<AudioState>,
    // Why the running tune is still retrying, for the UI
    tune_status: Arc<Mutex<Option<String>>>,
    tasks: Vec<JoinHandle<()>>,
}

impl WradioBackend {
    pub fn new(notify: Notifier) -> Self {
        Self {
            notify,
            device: None,
            audio: None,
            dynamic_label: Arc::new(Mutex::new(None)),
            slide: Arc::new(Mutex::new(None)),
            audio_state: Arc::new(AudioState::default()),
            tune_status: Arc::new(Mutex::new(None)),
            tasks: Vec::new(),
        }
    }

    fn device(&self) -> Result<&DeviceHandle, String> {
        self.device.as_ref().ok_or_else(|| "DAB device is not open".to_string())
    }

    fn set_label(notify: &Notifier, slot: &Mutex<Option<String>>, label: Option<String>) {
        *slot.lock().unwrap() = label;
        let _ = notify.send(());
    }

    fn set_slide(notify: &Notifier, slot: &Mutex<Option<PathBuf>>, slide: Option<PathBuf>) {
        *slot.lock().unwrap() = slide;
        let _ = notify.send(());
    }

    /// Sends a command that carries a completion channel and waits for the device task to
    /// actually finish it (a `Tune` can take several seconds to lock).
    async fn send_and_wait(
        device: &DeviceHandle,
        command: impl FnOnce(oneshot::Sender<wradio_c100_core::Result<()>>) -> Command,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        device.send(command(tx)).await.map_err(|e| e.to_string())?;
        rx.await
            .map_err(|_| "DAB device task stopped".to_string())?
            .map_err(|e| e.to_string())
    }

    fn report_audio(state: &AudioState, notify: &Notifier, alive: bool) {
        if alive {
            if !state.ready.swap(true, Ordering::SeqCst) {
                let _ = notify.send(());
            }
            return;
        }
        let mut error = state.error.lock().unwrap();
        if error.is_none() {
            *error = Some("Audio output stopped, see the log".to_string());
            state.ready.store(false, Ordering::SeqCst);
            let _ = notify.send(());
        }
    }

    /// ADTS-frames each access unit of `frame` and writes it to the audio output.
    async fn forward_audio(
        mut rx: broadcast::Receiver<AudioFrame>,
        audio: Arc<tokio::sync::Mutex<AudioOutput>>,
        state: Arc<AudioState>,
        notify: Notifier,
    ) {
        let mut adts_buf = Vec::new();
        // A summary line every few seconds instead of one per frame (~8 a second)
        let mut report = tokio::time::interval(AUDIO_REPORT_INTERVAL);
        report.tick().await;
        let (mut frames, mut bytes, mut dropped) = (0u32, 0usize, 0u32);
        let mut had_frames = false;
        let mut last_format = String::new();
        loop {
            let frame = tokio::select! {
                frame = rx.recv() => match frame {
                    Ok(frame) => frame,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = report.tick() => {
                    if frames > 0 {
                        eprintln!("[DAB] audio: {frames} frames, {bytes} bytes in {AUDIO_REPORT_INTERVAL:?} ({last_format}){}",
                            if dropped > 0 { format!(", {dropped} dropped (unusable sample rate)") } else { String::new() });
                        had_frames = true;
                    } else if let Some(since) = *state.waiting_since.lock().unwrap() {
                        eprintln!("[DAB] audio: no frames yet, {:?} after the station started", since.elapsed());
                    } else if had_frames {
                        eprintln!("[DAB] audio: frames stopped");
                        had_frames = false;
                    }
                    (frames, bytes, dropped) = (0, 0, 0);
                    continue;
                }
            };

            last_format = format!(
                "{} Hz, {} ch, sbr {}, ps {}, {} AUs",
                frame.sample_rate,
                frame.channels,
                frame.sbr_used,
                frame.ps_used,
                frame.access_unit_lengths.len()
            );
            if let Some(since) = state.waiting_since.lock().unwrap().take() {
                eprintln!("[DAB] first audio frame {:?} after the station started: {last_format}", since.elapsed());
            }

            // `sample_rate` is the post-SBR output rate; the ADTS header needs the AAC core's.
            let core_rate = if frame.sbr_used {
                frame.sample_rate / 2
            } else {
                frame.sample_rate
            };
            let Some(freq_idx) = adts::sampling_frequency_index(core_rate) else {
                dropped += 1;
                continue;
            };

            adts_buf.clear();
            let mut pos = 0usize;
            for &len in &frame.access_unit_lengths {
                let len = len as usize;
                let Some(au) = frame.access_units.get(pos..pos + len) else {
                    break;
                };
                adts::write_header(&mut adts_buf, len, freq_idx, frame.channels);
                adts_buf.extend_from_slice(au);
                pos += len;
            }
            frames += 1;
            bytes += adts_buf.len();

            if state.tuning.load(Ordering::SeqCst) > 0 {
                continue;
            }

            let alive = {
                let mut output = audio.lock().await;
                output.write_bytes(adts_buf.clone()).await;
                output.is_running()
            };
            Self::report_audio(&state, &notify, alive);
        }
    }

    fn stations_from(ensembles: HashMap<u32, EnsembleInfo>) -> Vec<DabStation> {
        let channels = Tuner::channel_table_khz();
        let mut ensembles: Vec<_> = ensembles.into_values().collect();
        ensembles.sort_by_key(|e| e.frequency_khz);

        ensembles
            .into_iter()
            .flat_map(|ensemble| {
                let channel = channels
                    .iter()
                    .position(|&khz| khz == ensemble.frequency_khz)
                    .unwrap_or_default() as u32;
                ensemble
                    .services
                    .into_iter()
                    .filter(|s| s.has_dab_plus_audio)
                    .map(move |s| DabStation {
                        id: s.service_id,
                        label: s.label.trim().to_string(),
                        channel,
                        frequency: ensemble.frequency_khz,
                        alternatives: Vec::new(),
                    })
            })
            .collect()
    }
}

impl WradioBackend {
    pub async fn open(&mut self) -> Result<(), String> {
        if self.device.is_some() {
            return Ok(());
        }

        let mut tuner = Tuner::open().await.map_err(|e| e.to_string())?;
        tuner.initialize().await.map_err(|e| e.to_string())?;
        let device = device::spawn(tuner);

        let mut audio = AudioOutput::new_adts(None);
        audio.start().await;
        let audio = Arc::new(tokio::sync::Mutex::new(audio));

        self.tasks.push(tokio::spawn(Self::forward_audio(
            device.subscribe_audio(),
            audio.clone(),
            self.audio_state.clone(),
            self.notify.clone(),
        )));

        let mut dls_rx = device.subscribe_dls_update();
        let notify = self.notify.clone();
        let label = self.dynamic_label.clone();
        self.tasks.push(tokio::spawn(async move {
            loop {
                match dls_rx.recv().await {
                    Ok(event) => {
                        let text = event.update.text.trim().to_string();
                        Self::set_label(&notify, &label, (!text.is_empty()).then_some(text));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }));

        // Slideshow pictures: shown as they arrive, and kept for the next time the station is tuned
        let mut slide_rx = device.subscribe_slideshow_update();
        let notify = self.notify.clone();
        let slide = self.slide.clone();
        self.tasks.push(tokio::spawn(async move {
            loop {
                match slide_rx.recv().await {
                    Ok(event) => {
                        let stored = tokio::task::spawn_blocking(move || {
                            let update = event.update;
                            slide_cache::store(event.service_id, &update.mime_type, &update.slideshow_data)
                        })
                        .await
                        .ok()
                        .flatten();
                        if let Some(path) = stored {
                            Self::set_slide(&notify, &slide, Some(path));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }));

        // The core refreshes this on every poll; only wake the UI when the bars actually move
        let mut lock_rx = device.subscribe_lock_status();
        let notify = self.notify.clone();
        self.tasks.push(tokio::spawn(async move {
            let mut last = None;
            loop {
                match lock_rx.recv().await {
                    Ok(quality) => {
                        let shown = (quality.locked, quality.signal_bars);
                        if last != Some(shown) {
                            last = Some(shown);
                            let _ = notify.send(());
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }));

        self.audio = Some(audio);
        self.device = Some(device);
        Ok(())
    }

    pub async fn close(&mut self) -> Result<(), String> {
        if let Some(device) = self.device.take() {
            // Best effort: the device may already be gone (unplugged).
            let _ = device.send(Command::StopService).await;
        }
        for task in self.tasks.drain(..) {
            task.abort();
        }
        if let Some(audio) = self.audio.take() {
            audio.lock().await.stop().await;
        }
        self.audio_state.reset();
        Self::set_label(&self.notify, &self.dynamic_label, None);
        Self::set_slide(&self.notify, &self.slide, None);
        Ok(())
    }

    pub async fn scan(
        &mut self,
        progress: &mut (dyn FnMut(DabScanProgress) + Send),
    ) -> Result<Vec<DabStation>, String> {
        let device = self.device()?.clone();
        let mut progress_rx = device.subscribe_scan_progress();
        let mut ensemble_rx = device.subscribe_ensemble_update();
        let mut snapshot_rx = device.watch_snapshot();
        device.send(Command::StartScan).await.map_err(|e| e.to_string())?;

        // An ensemble is reported repeatedly as its FIC fills in; the last one per frequency is
        // the most complete.
        let mut ensembles: HashMap<u32, EnsembleInfo> = HashMap::new();
        // Antenna level per frequency, to pick the best one of a station that is on several
        let mut signal: HashMap<u32, u8> = HashMap::new();
        let mut started = snapshot_rx.borrow().is_scanning;
        let start_deadline = tokio::time::Instant::now() + SCAN_START_TIMEOUT;
        let deadline = tokio::time::Instant::now() + SCAN_TIMEOUT;

        loop {
            tokio::select! {
                update = ensemble_rx.recv() => match update {
                    Ok(ensemble) => {
                        ensembles.insert(ensemble.frequency_khz, ensemble);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                update = progress_rx.recv() => match update {
                    Ok(p) => {
                        if let Some(bars) = p.signal_bars {
                            signal.insert(p.frequency_khz, bars);
                        }
                        progress(DabScanProgress {
                        channel_index: p.channel_index,
                        channel_count: p.channel_count,
                        frequency_khz: p.frequency_khz,
                        channel: channel_name(p.frequency_khz),
                        locked: p.locked,
                        failed: p.failed,
                        ensemble: p.ensemble_label,
                        service_count: p.service_count,
                    })
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                changed = snapshot_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let scanning = snapshot_rx.borrow().is_scanning;
                    if scanning {
                        started = true;
                    } else if started {
                        break;
                    }
                }
                _ = tokio::time::sleep_until(if started { deadline } else { start_deadline }) => {
                    if started {
                        let _ = device.send(Command::StopScan).await;
                        return Err("DAB scan timed out".into());
                    }
                    return Err("DAB scan did not start".into());
                }
            }
        }

        // Pick up any ensemble update that landed right before the scan finished.
        while let Ok(ensemble) = ensemble_rx.try_recv() {
            ensembles.insert(ensemble.frequency_khz, ensemble);
        }

        let mut stations = Self::stations_from(ensembles);
        stations.retain(|s| !s.label.is_empty());
        Ok(merge_duplicates(stations, &signal))
    }

    /// Everything a tune needs, owned, so it can run while the backend itself stays free (and be
    /// dropped to cancel it).
    pub fn tune_job(&self) -> Result<TuneJob, String> {
        *self.tune_status.lock().unwrap() = None;
        Ok(TuneJob {
            notify: self.notify.clone(),
            device: self.device()?.clone(),
            audio: self.audio.clone(),
            dynamic_label: self.dynamic_label.clone(),
            slide: self.slide.clone(),
            audio_state: self.audio_state.clone(),
            status: self.tune_status.clone(),
        })
    }

    pub fn tune_status(&self) -> Option<String> {
        self.tune_status.lock().unwrap().clone()
    }

    pub async fn stop_playback(&mut self) -> Result<(), String> {
        self.device()?
            .send(Command::StopService)
            .await
            .map_err(|e| e.to_string())?;
        self.audio_state.reset();
        Self::set_label(&self.notify, &self.dynamic_label, None);
        Self::set_slide(&self.notify, &self.slide, None);
        Ok(())
    }

    /// Signal bars (0..=6) of the tuned ensemble, if the device has reported any.
    pub fn signal_bars(&self) -> Option<u8> {
        self.device.as_ref()?.snapshot().lock_status?.signal_bars
    }

    pub fn audio_ready(&self) -> bool {
        self.audio_state.ready.load(Ordering::SeqCst)
    }

    pub fn audio_error(&self) -> Option<String> {
        self.audio_state.error.lock().unwrap().clone()
    }

    pub fn dynamic_label(&self) -> Option<String> {
        self.dynamic_label.lock().unwrap().clone()
    }

    pub fn slide(&self) -> Option<PathBuf> {
        self.slide.lock().unwrap().clone()
    }
}

/// One station tune. `run` keeps retrying until the station plays; dropping the future (the user
/// stopped, or picked another station) is how it's cancelled, so every wait in it is an await.
pub struct TuneJob {
    notify: Notifier,
    device: DeviceHandle,
    audio: Option<Arc<tokio::sync::Mutex<AudioOutput>>>,
    dynamic_label: Arc<Mutex<Option<String>>>,
    slide: Arc<Mutex<Option<PathBuf>>>,
    audio_state: Arc<AudioState>,
    status: Arc<Mutex<Option<String>>>,
}

/// Why a tune attempt didn't get the station playing.
enum Attempt {
    /// Might work next time (no lock, station list never arrived).
    Retry(String),
    Fatal(String),
}

impl TuneJob {
    fn set_status(&self, status: Option<String>) {
        *self.status.lock().unwrap() = status;
        let _ = self.notify.send(());
    }

    pub async fn run(&self, station: &DabStation) -> Result<(), String> {
        self.audio_state.reset();
        WradioBackend::set_label(&self.notify, &self.dynamic_label, None);
        // The station's last picture until the broadcast sends a new one
        WradioBackend::set_slide(&self.notify, &self.slide, slide_cache::latest(station.id));
        // The previous station's audio still sits in the output's buffers: flush them (which also
        // brings the output back if its pipeline had failed) and drop frames until this tune is done
        let _mute = TuneMute::new(&self.audio_state);
        if let Some(audio) = &self.audio {
            audio.lock().await.flush().await;
        }

        // The station's frequencies, best first: a frequency that won't lock hands over to the next
        let candidates: Vec<&DabStation> = std::iter::once(station).chain(&station.alternatives).collect();
        let mut candidate = 0;
        let mut tries = 0u32;
        let started = std::time::Instant::now();
        let mut attempt = 0u32;
        let mut force_retune = false;
        loop {
            attempt += 1;
            tries += 1;
            match self.attempt(candidates[candidate], force_retune, started).await {
                Ok(()) => {
                    self.set_status(None);
                    *self.audio_state.waiting_since.lock().unwrap() = Some(std::time::Instant::now());
                    return Ok(());
                }
                Err(Attempt::Retry(why)) => {
                    eprintln!("[DAB] {why} (attempt {attempt}), retrying");
                    self.set_status(Some(format!("{why}, retrying ({attempt})")));
                    if tries >= TRIES_PER_FREQUENCY && candidates.len() > 1 {
                        candidate = (candidate + 1) % candidates.len();
                        tries = 0;
                        eprintln!("[DAB] trying {} on {} kHz instead", station.label, candidates[candidate].frequency);
                    }
                    // A station list that never arrived needs a fresh tune, not the same one
                    force_retune = true;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(Attempt::Fatal(error)) => {
                    self.set_status(None);
                    return Err(error);
                }
            }
        }
    }

    async fn attempt(
        &self,
        station: &DabStation,
        force_retune: bool,
        started: std::time::Instant,
    ) -> Result<(), Attempt> {
        let channel = channel_name(station.frequency).unwrap_or("?");

        // Switching between services of the ensemble already tuned doesn't need a re-tune.
        let already_tuned = !force_retune
            && self
                .device
                .snapshot()
                .ensemble
                .is_some_and(|e| e.frequency_khz == station.frequency);
        eprintln!(
            "[DAB] tuning {} on {channel} ({} kHz){}",
            station.label,
            station.frequency,
            if already_tuned { ", ensemble already tuned" } else { "" }
        );
        if !already_tuned {
            self.tune_device(station.frequency).await.map_err(|error| match error {
                wradio_c100_core::Error::NoLock { .. } => Attempt::Retry(format!("No signal on {channel}")),
                other => Attempt::Fatal(other.to_string()),
            })?;
            eprintln!("[DAB] tuned after {:?}", started.elapsed());
        }

        // StartService is a silent no-op for a service the ensemble doesn't list yet, which is
        // the case right after a tune, so wait for the FIC to list it first.
        let mut snapshots = self.device.watch_snapshot();
        let listed = snapshots.wait_for(|snapshot| {
            snapshot.ensemble.as_ref().is_some_and(|e| {
                e.frequency_khz == station.frequency
                    && e.services.iter().any(|s| s.service_id == station.id && s.has_dab_plus_audio)
            })
        });
        match tokio::time::timeout(SERVICE_LIST_TIMEOUT, listed).await {
            Ok(Ok(_)) => eprintln!("[DAB] {} listed after {:?}", station.label, started.elapsed()),
            Ok(Err(_)) => return Err(Attempt::Fatal("DAB device task stopped".into())),
            Err(_) => return Err(Attempt::Retry(format!("Waiting for {channel}'s station list"))),
        }

        WradioBackend::send_and_wait(&self.device, |done| Command::StartService {
            service_id: station.id,
            done: Some(done),
        })
        .await
        .map_err(Attempt::Fatal)?;
        if self.device.snapshot().current_service_id != Some(station.id) {
            return Err(Attempt::Fatal(format!("{} could not be started", station.label)));
        }
        eprintln!("[DAB] {} started after {:?}", station.label, started.elapsed());
        Ok(())
    }

    /// `Command::Tune` with its error kept typed, so a missing lock can be told from the rest.
    async fn tune_device(&self, frequency_khz: u32) -> wradio_c100_core::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.device
            .send(Command::Tune {
                frequency_khz,
                done: Some(tx),
            })
            .await?;
        rx.await.map_err(|_| wradio_c100_core::Error::DeviceDisconnected)?
    }
}
