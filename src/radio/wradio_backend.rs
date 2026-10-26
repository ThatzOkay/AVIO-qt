use std::collections::HashMap;
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
use super::dab::{DabScanProgress, DabStation};

// A full Band III sweep waits on FIC collection per channel, so it takes a while on a weak
// antenna; this only guards against a scan that never reports finishing.
const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const SCAN_START_TIMEOUT: Duration = Duration::from_secs(5);

/// Drives the WRadio C100 (a generic Android USB DAB+ stick) through `wradio-c100-core`. Audio
/// leaves the core as raw DAB+ access units; they are ADTS-framed here and piped into GStreamer.
pub struct WradioBackend {
    notify: Notifier,
    device: Option<DeviceHandle>,
    audio: Option<Arc<tokio::sync::Mutex<AudioOutput>>>,
    dynamic_label: Arc<Mutex<Option<String>>>,
    tasks: Vec<JoinHandle<()>>,
}

impl WradioBackend {
    pub fn new(notify: Notifier) -> Self {
        Self {
            notify,
            device: None,
            audio: None,
            dynamic_label: Arc::new(Mutex::new(None)),
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

    /// ADTS-frames each access unit of `frame` and writes it to the audio output.
    async fn forward_audio(
        mut rx: broadcast::Receiver<AudioFrame>,
        audio: Arc<tokio::sync::Mutex<AudioOutput>>,
    ) {
        let mut adts_buf = Vec::new();
        loop {
            let frame = match rx.recv().await {
                Ok(frame) => frame,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            };

            // `sample_rate` is the post-SBR output rate; the ADTS header needs the AAC core's.
            let core_rate = if frame.sbr_used {
                frame.sample_rate / 2
            } else {
                frame.sample_rate
            };
            let Some(freq_idx) = adts::sampling_frequency_index(core_rate) else {
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

            audio.lock().await.write_bytes(adts_buf.clone()).await;
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
        Self::set_label(&self.notify, &self.dynamic_label, None);
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
                    Ok(p) => progress(DabScanProgress {
                        channel_index: p.channel_index,
                        channel_count: p.channel_count,
                        frequency_khz: p.frequency_khz,
                        channel: channel_name(p.frequency_khz),
                        locked: p.locked,
                        failed: p.failed,
                        ensemble: p.ensemble_label,
                        service_count: p.service_count,
                    }),
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
        Ok(stations)
    }

    pub async fn tune(&mut self, station: &DabStation) -> Result<(), String> {
        let device = self.device()?.clone();

        Self::set_label(&self.notify, &self.dynamic_label, None);

        // Switching between services of the ensemble already tuned doesn't need a re-tune.
        let already_tuned = device
            .snapshot()
            .ensemble
            .is_some_and(|e| e.frequency_khz == station.frequency);
        if !already_tuned {
            Self::send_and_wait(&device, |done| Command::Tune {
                frequency_khz: station.frequency,
                done: Some(done),
            })
            .await?;
        }

        Self::send_and_wait(&device, |done| Command::StartService {
            service_id: station.id,
            done: Some(done),
        })
        .await
    }

    pub async fn stop_playback(&mut self) -> Result<(), String> {
        self.device()?
            .send(Command::StopService)
            .await
            .map_err(|e| e.to_string())?;
        Self::set_label(&self.notify, &self.dynamic_label, None);
        Ok(())
    }

    pub fn dynamic_label(&self) -> Option<String> {
        self.dynamic_label.lock().unwrap().clone()
    }
}
