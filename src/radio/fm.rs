use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rtl_sdr::fm::fm_radio::{
    RdsInfo, fm_close, fm_get_rds, fm_open, fm_read, fm_set_frequency, fm_set_gain,
    fm_set_sample_rate, fm_stop,
};
use serde::Serialize;
use tokio::sync::mpsc;

use super::Notifier;
use crate::audio::audio_output::{AudioOutput, AudioOutputMode, AudioOutputOptions};

pub const FM_FAVORITE_SLOTS: usize = 5;
pub const DEFAULT_FREQUENCY_KHZ: u32 = 100_000;
pub const FM_BAND_MIN_KHZ: u32 = 87_000;
pub const FM_BAND_MAX_KHZ: u32 = 108_000;

const SAMPLE_RATE: u32 = 2_048_000;
const OUTPUT_RATE: u32 = 48_000;
const FM_STEP_KHZ: u32 = 50;
const FM_FAST_STEP_KHZ: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StationInfo {
    pub program_id: u16,
    pub genre: String,
    pub name: Option<String>,
    pub text: Option<String>,
}

pub struct FmTuner {
    device_open: bool,
    running: bool,
    frequency: u32,
    audio_output: Option<Arc<tokio::sync::Mutex<AudioOutput>>>,
    // Updated by the task draining the FFI read callback, outside any &mut self call
    station: Arc<Mutex<Option<StationInfo>>>,
    favorites: [u32; FM_FAVORITE_SLOTS],
    notify: Notifier,
}

impl FmTuner {
    pub fn new(notify: Notifier, frequency: u32, favorites: &[u32]) -> Self {
        let mut slots = [0; FM_FAVORITE_SLOTS];
        for (slot, &khz) in slots.iter_mut().zip(favorites) {
            *slot = khz;
        }
        Self {
            device_open: false,
            running: false,
            frequency: clamp(frequency),
            audio_output: None,
            station: Arc::new(Mutex::new(None)),
            favorites: slots,
            notify,
        }
    }

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn frequency(&self) -> u32 {
        self.frequency
    }

    pub fn favorites(&self) -> &[u32] {
        &self.favorites
    }

    pub fn station(&self) -> Option<StationInfo> {
        self.station.lock().unwrap().clone()
    }

    pub async fn start(&mut self, frequency: u32) -> Result<(), String> {
        let frequency = clamp(frequency);
        if self.running {
            self.tune(frequency).await;
            return Ok(());
        }

        if !self.device_open {
            fm_open(0).await.map_err(|e| format!("Failed to open the RTL-SDR: {e}"))?;
            self.device_open = true;
            fm_set_sample_rate(SAMPLE_RATE).await;
            fm_set_gain(200).await;
        }

        self.frequency = frequency;
        fm_set_frequency(frequency * 1000).await;

        let mut audio_output = AudioOutput::new(AudioOutputOptions {
            sample_rate: OUTPUT_RATE as i32,
            channels: 1,
            mode: Some(AudioOutputMode::Music),
            device: None,
        });
        audio_output.start().await;
        let audio_output = Arc::new(tokio::sync::Mutex::new(audio_output));
        self.audio_output = Some(audio_output.clone());

        // rtlsdr_read_async calls this from its own thread and stalls USB draining if it blocks,
        // so it only forwards the buffer; a task does the actual writing and RDS bookkeeping
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<f32>>();
        let send_failed_once = Arc::new(AtomicBool::new(false));
        let started = fm_read(
            Box::new(move |buff: Vec<f32>| {
                if tx.send(buff).is_err() && !send_failed_once.swap(true, Ordering::Relaxed) {
                    eprintln!("FM audio channel send failed (receiver dropped)");
                }
            }),
            OUTPUT_RATE,
        )
        .await
        .map_err(|e| e.to_string());
        if let Err(e) = started {
            self.release().await;
            return Err(format!("Failed to start FM read: {e}"));
        }

        let station = self.station.clone();
        let notify = self.notify.clone();
        crate::runtime::spawn(async move {
            while let Some(buff) = rx.recv().await {
                audio_output.lock().await.write(pcm_from_f32(&buff)).await;

                let next = to_station_info(fm_get_rds());
                let changed = {
                    let mut current = station.lock().unwrap();
                    let changed = *current != next;
                    if changed {
                        *current = next;
                    }
                    changed
                };
                if changed {
                    let _ = notify.send(());
                }
            }
        });

        self.running = true;
        Ok(())
    }

    pub async fn stop(&mut self) {
        if !self.running && !self.device_open {
            return;
        }
        self.release().await;
    }

    async fn release(&mut self) {
        if self.device_open {
            fm_stop().await;
            fm_close().await;
            self.device_open = false;
        }
        self.running = false;
        *self.station.lock().unwrap() = None;
        if let Some(audio_output) = self.audio_output.take() {
            audio_output.lock().await.stop().await;
        }
    }

    pub async fn step(&mut self, direction: i32, fast: bool) {
        let step = i64::from(if fast { FM_FAST_STEP_KHZ } else { FM_STEP_KHZ });
        let next = (i64::from(self.frequency) + step * i64::from(direction)).max(0) as u32;
        self.tune(clamp(next)).await;
    }

    pub fn set_favorite(&mut self, slot: usize) -> Result<(), String> {
        let favorite = self
            .favorites
            .get_mut(slot)
            .ok_or_else(|| format!("Favorite slot {slot} out of range"))?;
        *favorite = self.frequency;
        Ok(())
    }

    pub async fn recall_favorite(&mut self, slot: usize) -> Result<(), String> {
        let frequency = match self.favorites.get(slot) {
            Some(&khz) if khz > 0 => khz,
            _ => return Err(format!("Favorite slot {slot} is empty")),
        };
        self.start(frequency).await
    }

    async fn tune(&mut self, frequency: u32) {
        self.frequency = frequency;
        if self.running && self.device_open {
            fm_set_frequency(frequency * 1000).await;
            *self.station.lock().unwrap() = None;
        }
    }
}

fn clamp(khz: u32) -> u32 {
    khz.clamp(FM_BAND_MIN_KHZ, FM_BAND_MAX_KHZ)
}

fn to_station_info(rds: RdsInfo) -> Option<StationInfo> {
    if rds.program_id == 0 && rds.program_type == "0" {
        return None;
    }

    Some(StationInfo {
        program_id: rds.program_id as u16,
        genre: rds.program_type,
        name: rds.station_name,
        text: rds.radio_text,
    })
}

fn pcm_from_f32(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect()
}
