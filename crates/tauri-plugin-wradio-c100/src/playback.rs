//! Native audio playback: decodes the device task's raw AAC access units
//! and plays the resulting PCM through the speakers. Per the Tauri plugin
//! section: "don't stream raw AAC/PCM to the webview... play it natively
//! in the Rust backend with cpal and expose only transport controls" --
//! `commands.rs`'s `set_volume`/`mute` are those controls.
//!
//! Decoding goes through an `ffmpeg` subprocess (ADTS in via stdin,
//! `f32le` PCM out via stdout) rather than a Rust AAC crate. `syom` (a
//! pure-Rust HE-AAC decoder) was tried first but has a real bug with
//! DAB+'s implicit SBR streaming pattern: a single `Decoder` instance
//! decodes the first access unit fine, then permanently corrupts itself
//! the moment it hits the normal mid-stream SBR rate upgrade. `ffmpeg`'s
//! own AAC decoder already proved itself flawless against this exact
//! broadcast content earlier in this port (`--dump-aac | ffplay`,
//! `ffprobe`), so this reuses that proven path instead.
//!
//! # Architecture
//!
//! - A tokio task ("decode task") subscribes to
//!   [`DeviceHandle::subscribe_audio`]. On the first [`AudioFrame`] (or
//!   whenever `(sample_rate, channels)` changes -- a station switch), it
//!   (re)spawns ffmpeg with that exact output format and writes each
//!   ADTS-wrapped access unit (`wradio_c100_core::adts`) to its stdin.
//! - A second task per ffmpeg instance reads `f32le` samples from its
//!   stdout in batches and pushes them into an `rtrb` lock-free ring
//!   buffer.
//! - A plain OS thread owns `cpal`'s `Host`/`Device`/`Stream` --
//!   deliberately not a tokio task, since `cpal::Stream` isn't `Send` on
//!   every backend and can't cross an `.await` point.
//! - A fresh ring buffer accompanies each format change rather than being
//!   reused: `rtrb::Consumer` isn't `Clone`, and a rebuilt `cpal` stream
//!   needs to own one for its whole lifetime.
//!
//! Volume/mute are read inside `cpal`'s realtime callback via
//! `Arc<AtomicU32>`/`Arc<AtomicBool>`, never a mutex there.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::broadcast;

use wradio_c100_core::adts;
use wradio_c100_core::device::{AudioFrame, DeviceHandle};

/// ~2 seconds of 48kHz stereo of slack between the decode task's (bursty)
/// and the audio thread's (steady) rates.
const RING_BUFFER_CAPACITY: usize = 48_000 * 2 * 2;

/// Batch size for reading ffmpeg's stdout -- reading one 4-byte sample at
/// a time (an earlier version did) means ~96,000 tiny async reads/sec at
/// 48kHz stereo, real overhead that caused audible popping (the reader
/// task falling behind, starving the ring buffer) before this was batched.
const READ_CHUNK_SAMPLES: usize = 1024;

/// Handle to a running playback pipeline. Cheap to clone (both fields are
/// `Arc`s); stored in `tauri::State` alongside [`DeviceHandle`].
#[derive(Clone)]
pub struct PlaybackHandle {
    volume_bits: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
}

impl PlaybackHandle {
    pub fn set_volume(&self, volume: f32) {
        self.volume_bits.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }
}

/// One decoded PCM format, paired with the ring buffer `Consumer` a fresh
/// `cpal` stream for it should read from.
struct FormatChange {
    sample_rate: u32,
    channels: u16,
    consumer: rtrb::Consumer<f32>,
}

/// Spawns the decode task + audio thread and returns a [`PlaybackHandle`].
pub fn spawn(device: &DeviceHandle) -> PlaybackHandle {
    tracing::info!("playback: spawning decode task + audio thread");
    let volume_bits = Arc::new(AtomicU32::new(1.0f32.to_bits()));
    let muted = Arc::new(AtomicBool::new(false));

    let (format_tx, format_rx) = std::sync::mpsc::channel::<FormatChange>();
    spawn_audio_thread(format_rx, volume_bits.clone(), muted.clone());
    spawn_decode_task(device.subscribe_audio(), format_tx);

    PlaybackHandle { volume_bits, muted }
}

fn spawn_ffmpeg(
    sample_rate: u32,
    channels: u16,
) -> std::io::Result<(Child, ChildStdin, tokio::process::ChildStdout)> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            // "aac" is this ffmpeg build's demuxer name for raw ADTS
            // input; "adts" is only the muxer/output-side name.
            "-f",
            "aac",
            "-i",
            "pipe:0",
            "-f",
            "f32le",
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            &channels.to_string(),
            "pipe:1",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    tauri::async_runtime::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr);
        let mut line = String::new();
        while tokio::io::AsyncBufReadExt::read_line(&mut lines, &mut line)
            .await
            .is_ok_and(|n| n > 0)
        {
            tracing::warn!(line = line.trim(), "ffmpeg stderr");
            line.clear();
        }
    });

    Ok((child, stdin, stdout))
}

fn spawn_ffmpeg_stdout_reader(
    mut stdout: tokio::process::ChildStdout,
    mut producer: rtrb::Producer<f32>,
) {
    tauri::async_runtime::spawn(async move {
        let mut buf = [0u8; READ_CHUNK_SAMPLES * 4];
        loop {
            if let Err(err) = stdout.read_exact(&mut buf).await {
                if err.kind() != std::io::ErrorKind::UnexpectedEof {
                    tracing::warn!(?err, "playback: ffmpeg stdout read error");
                }
                return;
            }
            for chunk in buf.chunks_exact(4) {
                // Ring buffer full -- drop the sample rather than block;
                // a realtime audio path drops on overflow rather than
                // stalling the whole pipeline.
                let _ = producer.push(f32::from_le_bytes(chunk.try_into().unwrap()));
            }
        }
    });
}

fn spawn_decode_task(
    mut audio_rx: broadcast::Receiver<AudioFrame>,
    format_tx: std::sync::mpsc::Sender<FormatChange>,
) {
    // `tauri::async_runtime::spawn`, not bare `tokio::spawn`: Tauri
    // already runs a tokio runtime and provides this wrapper so plugins
    // don't start a second one.
    tauri::async_runtime::spawn(async move {
        let mut current: Option<((u32, u16), Child, ChildStdin)> = None;
        let mut adts_scratch = Vec::new();

        loop {
            let frame = match audio_rx.recv().await {
                Ok(frame) => frame,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, "playback decode task lagged, some audio frames dropped");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::info!("playback: audio channel closed, decode task exiting");
                    return;
                }
            };

            let target = (frame.sample_rate, frame.channels as u16);
            if current.as_ref().map(|(fmt, ..)| *fmt) != Some(target) {
                tracing::info!(
                    sample_rate = target.0,
                    channels = target.1,
                    "playback: format changed, restarting ffmpeg"
                );
                match spawn_ffmpeg(target.0, target.1) {
                    Ok((child, stdin, stdout)) => {
                        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING_BUFFER_CAPACITY);
                        spawn_ffmpeg_stdout_reader(stdout, producer);
                        let _ = format_tx.send(FormatChange {
                            sample_rate: target.0,
                            channels: target.1,
                            consumer,
                        });
                        current = Some((target, child, stdin));
                    }
                    Err(err) => {
                        tracing::warn!(?err, "playback: failed to spawn ffmpeg");
                        current = None;
                        continue;
                    }
                }
            }

            let Some((_, _child, stdin)) = &mut current else {
                continue;
            };

            let mut pos = 0usize;
            for &len in &frame.access_unit_lengths {
                let len = len as usize;
                let au = &frame.access_units[pos..pos + len];
                pos += len;

                let core_rate = if frame.sbr_used {
                    frame.sample_rate / 2
                } else {
                    frame.sample_rate
                };
                let Some(freq_idx) = adts::sampling_frequency_index(core_rate) else {
                    continue;
                };
                adts_scratch.clear();
                adts::write_header(&mut adts_scratch, len, freq_idx, frame.channels);
                adts_scratch.extend_from_slice(au);

                if let Err(err) = stdin.write_all(&adts_scratch).await {
                    tracing::warn!(?err, "playback: ffmpeg stdin write failed, will respawn");
                    current = None;
                    break;
                }
            }
        }
    });
}

fn spawn_audio_thread(
    format_rx: std::sync::mpsc::Receiver<FormatChange>,
    volume_bits: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let host = cpal::default_host();
        // Keeps the current `Stream` alive across format changes --
        // `cpal` plays for as long as the value isn't dropped.
        let mut _stream: Option<cpal::Stream> = None;

        for change in format_rx {
            let Some(device) = host.default_output_device() else {
                tracing::warn!("no default audio output device, can't play decoded audio");
                continue;
            };
            let config = cpal::StreamConfig {
                channels: change.channels,
                sample_rate: change.sample_rate,
                buffer_size: cpal::BufferSize::Default,
            };
            let mut consumer = change.consumer;
            let volume_bits = volume_bits.clone();
            let muted = muted.clone();
            let stream = device.build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let volume = f32::from_bits(volume_bits.load(Ordering::Relaxed));
                    let is_muted = muted.load(Ordering::Relaxed);
                    for sample in data.iter_mut() {
                        let decoded = consumer.pop().unwrap_or(0.0);
                        *sample = if is_muted { 0.0 } else { decoded * volume };
                    }
                },
                |err| tracing::warn!(?err, "cpal output stream error"),
                None,
            );
            match stream {
                Ok(stream) => match stream.play() {
                    Ok(()) => {
                        tracing::info!(
                            sample_rate = change.sample_rate,
                            channels = change.channels,
                            "playback: cpal output stream built and playing"
                        );
                        _stream = Some(stream);
                    }
                    Err(err) => tracing::warn!(?err, "failed to start cpal output stream"),
                },
                Err(err) => {
                    tracing::warn!(
                        ?err,
                        sample_rate = change.sample_rate,
                        channels = change.channels,
                        "failed to build cpal output stream for this format"
                    );
                }
            }
        }
    });
}
