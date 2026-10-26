//! Async "device task": owns a [`Tuner`] and runs the equivalent of the
//! reference's `commandProcessing`/`readData` background loops as one
//! `tokio` task, per `RUST_PORT_PROMPT.md`'s Async/runtime section --
//! replacing `ConcurrentQueue`/`CallbackDispatcher`
//! (`concurrent_queue.h`/`callback.h`/`callbackhandle.h`) with
//! `mpsc`/`broadcast`/`watch` channels instead of porting those files
//! literally.
//!
//! This is the layer `tauri-plugin-wradio-c100` (Phase 8) is built on:
//! its `Builder::setup` spawns [`spawn`] once and stores the resulting
//! [`DeviceHandle`] as managed `tauri::State`, wiring each `subscribe_*`
//! receiver to `app_handle.emit(...)` and each command to a
//! `#[tauri::command]`. `examples/scan_and_listen.rs` (Phases 1-7)
//! deliberately does *not* go through this -- it drives [`Tuner`]
//! directly for simpler single-shot CLI iteration, and both stay working
//! side by side (`RUST_PORT_PROMPT.md` explicitly asks for both).
//!
//! # Cancel-safety
//!
//! The task loop never races a [`Command`] against an in-flight USB
//! transaction. `Hal::read_register`/`read_msc_data`/etc. are each a
//! `bulk_write` (send the request) immediately followed by a `bulk_read`
//! (fetch the reply) -- two separate `.await` points, not one atomic USB
//! transaction -- so a `tokio::select!` that could cancel
//! `Tuner::poll_msc()`/`poll_fic()` mid-transaction desyncs the device's
//! reply stream for whatever unrelated read comes next. This was found
//! and fixed the hard way building `examples/scan_and_listen.rs`'s own
//! interactive session loop (see `PORT_CHECKLIST.md`'s write-up right
//! before this phase); the device task here is written to that same rule
//! from the start: every step below either checks for a new command with
//! a non-blocking `try_recv()` *between* steps, or -- when idle, with no
//! I/O in flight to protect -- blocks on `cmd_rx.recv()` alone.

use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::announcements::{AnnouncementEvent, AnnouncementSwitcher};
use crate::dabplus::{self, DabPlusDecoder};
use crate::dls::DlsUpdate;
use crate::ensemble::Ensemble;
use crate::fic;
use crate::pad;
use crate::service::ServiceComponentKind;
use crate::slideshow::SlideshowUpdate;
use crate::tables;
use crate::tuner::{SignalQuality, Tuner};
use crate::Error;

/// Broadcast channel capacity for event streams -- generous enough that a
/// slow/momentarily-disconnected subscriber (a webview event listener
/// that hasn't been (re)attached yet) doesn't cause `RecvError::Lagged`
/// under normal event rates (DLS/slideshow/lock-status are all at most a
/// few events per second).
const EVENT_CHANNEL_CAPACITY: usize = 64;

/// Commands accepted by the device task, replacing the reference's
/// `m_commandQueue`/`m_scanCommandQueue` (`ConcurrentQueue<std::function<void()>>`).
///
/// Not `Clone` -- `Tune`/`StartService`'s `done` sender is inherently
/// single-use (that's the whole point of a `oneshot`), and nothing in
/// this crate ever cloned a `Command` anyway.
#[derive(Debug)]
pub enum Command {
    /// Tune to a frequency and start collecting its FIC (mirrors
    /// `RaonTunerInput::tuneFrequency` + FIC collection loop). Stops any
    /// currently-open service/scan first.
    Tune {
        frequency_khz: u32,
        /// Signaled once this command has actually finished being
        /// processed (tuned + locked, or failed) -- `None` for callers
        /// that only care it was queued (`DeviceHandle::send`'s own doc
        /// comment on why that alone isn't "it happened"). Added
        /// specifically because the Tauri plugin's `tune` command used to
        /// resolve as soon as this was *enqueued*, several real seconds
        /// before `tune_and_wait_for_lock` actually finished -- a
        /// frontend "still tuning" indicator needs the real completion,
        /// not the enqueue ack.
        done: Option<oneshot::Sender<crate::Result<()>>>,
    },
    /// Start decoding a service's audio/PAD on the currently-tuned
    /// ensemble (mirrors `setService`) -- `Tune` first if nothing's
    /// tuned yet. No-op if `service_id` isn't a known DAB+ audio service
    /// on the current ensemble (the caller finds out via the next
    /// `Snapshot`/`ensemble_update`, not a command error, matching the
    /// reference's own fire-and-forget command-queue shape) -- `done`
    /// (see `Tune`'s own doc comment) still resolves in that case, just
    /// with `Ok(())`, since silently no-op'ing isn't itself an error.
    StartService {
        service_id: u32,
        done: Option<oneshot::Sender<crate::Result<()>>>,
    },
    /// Stop decoding the current service and close its subchannel.
    /// Leaves the ensemble tuned/FIC-collecting.
    StopService,
    /// Sweep the full Band III channel table (mirrors `scanNext`/
    /// `processScanCommands`), emitting one [`ScanProgress`] per channel
    /// and one [`EnsembleInfo`] update for each that locks. Stops any
    /// currently-open service first.
    StartScan,
    /// Stop an in-progress scan after the current channel's FIC
    /// collection step finishes (checked between channels, same
    /// non-racing rule as everywhere else in this module -- never mid
    /// register transaction).
    StopScan,
}

/// A clean IPC-ready projection of [`crate::service::Service`] -- kept as
/// its own DTO (named to match `RUST_PORT_PROMPT.md`'s `ServiceInfo`)
/// rather than serializing `Service` directly, since that type also
/// carries `component_keys: Vec<ComponentKey>` -- an internal addressing
/// detail into the owning [`Ensemble`]'s component maps that's meaningless
/// off in a webview.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    pub service_id: u32,
    pub label: String,
    pub programme_type_code: u8,
    /// Whether this service has a DAB+ (ASCTY 0x3F) audio stream
    /// component with a resolved subchannel -- i.e. whether
    /// `Command::StartService { service_id }` would actually do
    /// anything. Classic (MPEG Layer II, `ascty == 0x00`) audio and
    /// data-only services are never playable here, matching
    /// `dabplus.rs`'s explicit scope (`RUST_PORT_PROMPT.md`).
    pub has_dab_plus_audio: bool,
}

fn service_info(ensemble: &Ensemble, service: &crate::service::Service) -> ServiceInfo {
    ServiceInfo {
        service_id: service.service_id,
        label: service.label.clone(),
        programme_type_code: service.programme_type_code,
        has_dab_plus_audio: ensemble.service_has_dab_plus_audio(service),
    }
}

/// A clean IPC-ready projection of [`Ensemble`] (`RUST_PORT_PROMPT.md`'s
/// `EnsembleInfo`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnsembleInfo {
    pub ensemble_id: u16,
    pub label: String,
    pub ecc: u8,
    pub frequency_khz: u32,
    pub services: Vec<ServiceInfo>,
}

fn ensemble_info(ensemble: &Ensemble) -> EnsembleInfo {
    EnsembleInfo {
        ensemble_id: ensemble.ensemble_id(),
        label: ensemble.ensemble_label().to_string(),
        ecc: ensemble.ensemble_ecc(),
        frequency_khz: ensemble.frequency_khz(),
        services: ensemble
            .services()
            .map(|s| service_info(ensemble, s))
            .collect(),
    }
}

/// One channel's result during a [`Command::StartScan`] sweep.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub channel_index: usize,
    pub channel_count: usize,
    pub frequency_khz: u32,
    pub locked: bool,
    pub failed: bool,
    /// `Some` (and the corresponding `ensemble_update` event also fires)
    /// only once this channel's ensemble id (FIG 0/0) was actually
    /// resolved within the per-channel collection budget -- mirrors
    /// `Store::upsert_station`'s own 0xFFFF-sentinel guard (Phase 7's
    /// real-hardware bug -- see `PORT_CHECKLIST.md`), applied here so a
    /// scan consumer never sees a bogus unresolved-ensemble entry either.
    pub ensemble_label: Option<String>,
    pub service_count: usize,
}

/// One Dynamic Label update, tagged with which service it came from (a
/// UI/plugin subscribed to a single broadcast stream needs this to know
/// whether it's still relevant to what's currently selected).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DlsEvent {
    pub service_id: u32,
    pub update: DlsUpdate,
}

/// One slideshow update, tagged the same way as [`DlsEvent`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideshowEvent {
    pub service_id: u32,
    pub update: SlideshowUpdate,
}

/// One superframe's decoded audio, for a native playback layer (`cpal`,
/// per `RUST_PORT_PROMPT.md`'s explicit "don't stream raw AAC/PCM to the
/// webview" instruction -- this channel is for a Rust-side consumer, not
/// forwarded to JS). No decoder is wired up in this crate (see
/// `dabplus.rs`'s own doc comment on the open AAC-decoder-licensing
/// question) -- this only carries the already-existing raw access units
/// out of the device task so *something* downstream can decode them
/// later; not `Serialize` (never crosses IPC).
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub service_id: u32,
    pub access_units: Vec<u8>,
    pub access_unit_lengths: Vec<u16>,
    pub channels: u8,
    pub sample_rate: u32,
    pub sbr_used: bool,
    pub ps_used: bool,
}

/// Latest-value-wins snapshot of device state -- the `watch` channel
/// `RUST_PORT_PROMPT.md` calls for ("current ensemble/service DB
/// snapshot" and "current lock status"), and what `get_snapshot` (Phase
/// 8's command) returns directly.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub ensemble: Option<EnsembleInfo>,
    pub lock_status: Option<SignalQuality>,
    pub failed_status: bool,
    pub current_service_id: Option<u32>,
    pub is_scanning: bool,
}

struct Outputs {
    scan_progress: broadcast::Sender<ScanProgress>,
    ensemble_update: broadcast::Sender<EnsembleInfo>,
    lock_status: broadcast::Sender<SignalQuality>,
    dls_update: broadcast::Sender<DlsEvent>,
    slideshow_update: broadcast::Sender<SlideshowEvent>,
    announcement: broadcast::Sender<AnnouncementEvent>,
    audio: broadcast::Sender<AudioFrame>,
    snapshot: watch::Sender<Snapshot>,
}

/// A handle to a running device task: send [`Command`]s, subscribe to its
/// output event streams, or read/watch its latest [`Snapshot`]. Cloning
/// this is cheap (every field is a channel handle) and every clone talks
/// to the same underlying task -- the natural shape for `tauri::State`
/// (shared across every command invocation) or multiple independent
/// example/test callers.
#[derive(Clone)]
pub struct DeviceHandle {
    cmd_tx: mpsc::Sender<Command>,
    out: std::sync::Arc<Outputs>,
}

impl DeviceHandle {
    /// Sends a command to the device task. Errors only if the task has
    /// stopped (e.g. panicked) -- commands themselves are fire-and-forget
    /// (see [`Command`]'s own doc comments for why), so success here just
    /// means the task is alive to receive it, not that it necessarily did
    /// anything (e.g. `StartService` for an unknown id).
    pub async fn send(&self, command: Command) -> crate::Result<()> {
        self.cmd_tx
            .send(command)
            .await
            .map_err(|_| Error::DeviceDisconnected)
    }

    /// The latest [`Snapshot`], without waiting for a change.
    pub fn snapshot(&self) -> Snapshot {
        self.out.snapshot.borrow().clone()
    }

    /// A `watch` receiver over [`Snapshot`] -- `.changed().await` then
    /// `.borrow()` for "wake me on the next change", or `.borrow()`
    /// directly for "give me the latest value now" (same as
    /// [`DeviceHandle::snapshot`]).
    pub fn watch_snapshot(&self) -> watch::Receiver<Snapshot> {
        self.out.snapshot.subscribe()
    }

    pub fn subscribe_scan_progress(&self) -> broadcast::Receiver<ScanProgress> {
        self.out.scan_progress.subscribe()
    }

    pub fn subscribe_ensemble_update(&self) -> broadcast::Receiver<EnsembleInfo> {
        self.out.ensemble_update.subscribe()
    }

    pub fn subscribe_lock_status(&self) -> broadcast::Receiver<SignalQuality> {
        self.out.lock_status.subscribe()
    }

    pub fn subscribe_dls_update(&self) -> broadcast::Receiver<DlsEvent> {
        self.out.dls_update.subscribe()
    }

    pub fn subscribe_slideshow_update(&self) -> broadcast::Receiver<SlideshowEvent> {
        self.out.slideshow_update.subscribe()
    }

    pub fn subscribe_announcement(&self) -> broadcast::Receiver<AnnouncementEvent> {
        self.out.announcement.subscribe()
    }

    /// Decoded audio, for a native (`cpal`) playback layer -- see
    /// [`AudioFrame`]'s own doc comment on why this never crosses IPC.
    pub fn subscribe_audio(&self) -> broadcast::Receiver<AudioFrame> {
        self.out.audio.subscribe()
    }
}

/// Spawns the device task owning `tuner` and returns a [`DeviceHandle`]
/// to it. The task runs until every [`DeviceHandle`] clone (and its
/// internal `cmd_tx`) is dropped.
pub fn spawn(tuner: Tuner) -> DeviceHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (snapshot_tx, _snapshot_rx) = watch::channel(Snapshot::default());
    let out = std::sync::Arc::new(Outputs {
        scan_progress: broadcast::channel(EVENT_CHANNEL_CAPACITY).0,
        ensemble_update: broadcast::channel(EVENT_CHANNEL_CAPACITY).0,
        lock_status: broadcast::channel(EVENT_CHANNEL_CAPACITY).0,
        dls_update: broadcast::channel(EVENT_CHANNEL_CAPACITY).0,
        slideshow_update: broadcast::channel(EVENT_CHANNEL_CAPACITY).0,
        announcement: broadcast::channel(EVENT_CHANNEL_CAPACITY).0,
        audio: broadcast::channel(EVENT_CHANNEL_CAPACITY).0,
        snapshot: snapshot_tx,
    });

    let task_out = out.clone();
    tokio::spawn(async move {
        run(tuner, cmd_rx, task_out).await;
    });

    DeviceHandle { cmd_tx, out }
}

/// Per-service decode state, live only while a service is selected
/// (`Command::StartService`..`StopService`/`Tune`/another `StartService`).
struct ServiceSession {
    service_id: u32,
    /// The subchannel currently open for this service -- either its
    /// primary programme's, or an announcement's for the duration of an
    /// interruption (see [`AnnouncementSwitcher`]). Tracked here
    /// separately from `Tuner`'s own internal `current_subchan_id`
    /// because this module needs to *compare* the two ids to decide
    /// whether an `AnnouncementSwitcher::update` result actually requires
    /// closing/reopening anything, not just remember the current one.
    open_subchannel_id: u8,
    bitrate_kbits: u16,
    decoder: DabPlusDecoder,
    pad_decoder: pad::PadDecoder,
    last_dls_text: String,
    announcements: AnnouncementSwitcher,
}

struct TaskState {
    tuner: Tuner,
    ensemble: Ensemble,
    tuned: bool,
    service: Option<ServiceSession>,
    scanning: bool,
    /// The most recent [`SignalQuality`] sent on `out.lock_status`, kept
    /// here purely so [`TaskState::publish_snapshot`] has a value to put
    /// in [`Snapshot::lock_status`] -- `broadcast::Sender` has no "latest
    /// value" replay for a receiver that (re)subscribes after the fact
    /// (unlike `watch`), so reading it back via a fresh `subscribe()` +
    /// `try_recv()` (an earlier draft of this module did exactly that)
    /// would almost always see nothing, silently leaving every snapshot's
    /// `lock_status` `None`.
    last_signal_quality: Option<SignalQuality>,
    failed: bool,
}

impl TaskState {
    fn new(tuner: Tuner) -> Self {
        Self {
            tuner,
            ensemble: Ensemble::new(),
            tuned: false,
            service: None,
            scanning: false,
            last_signal_quality: None,
            failed: false,
        }
    }

    fn publish_snapshot(&self, out: &Outputs) {
        out.snapshot.send_replace(Snapshot {
            ensemble: self.tuned.then(|| ensemble_info(&self.ensemble)),
            lock_status: self.last_signal_quality,
            failed_status: self.failed,
            current_service_id: self.service.as_ref().map(|s| s.service_id),
            is_scanning: self.scanning,
        });
    }

    /// Closes the current service's subchannel, if any, and drops its
    /// decode state. Leaves `self.tuned`/`self.ensemble` alone.
    async fn stop_service(&mut self) -> crate::Result<()> {
        if self.service.is_some() {
            self.tuner.close_subchannel().await?;
            self.service = None;
        }
        Ok(())
    }

    /// Body of [`Command::Tune`], factored out so [`Self::handle_command`]
    /// can capture its `Result` (to log it and/or notify `done`) instead
    /// of just propagating it via `?` the way every other command does --
    /// see `Command::Tune`'s own doc comment on why this one specifically
    /// needs that.
    async fn do_tune(&mut self, frequency_khz: u32, out: &Outputs) -> crate::Result<()> {
        self.scanning = false;
        self.stop_service().await?;
        self.tuner
            .tune_and_wait_for_lock(frequency_khz, std::time::Duration::from_secs(4), 3)
            .await?;
        self.ensemble = Ensemble::new();
        self.ensemble.set_frequency_khz(frequency_khz);
        self.tuned = true;
        self.publish_snapshot(out);
        Ok(())
    }

    /// Starts decoding `service_id` on the current ensemble, closing
    /// whatever was open first. No-op (not an error) if `service_id`
    /// isn't a currently-known DAB+ audio service -- see
    /// [`Command::StartService`]'s own doc comment.
    async fn start_service(&mut self, service_id: u32, out: &Outputs) -> crate::Result<()> {
        let Some(service) = self.ensemble.service(service_id) else {
            return Ok(());
        };
        let Some((subchannel_id, bitrate_kbits)) = service.component_keys.iter().find_map(|&key| {
            let component = self.ensemble.component(key)?;
            match component.kind {
                ServiceComponentKind::StreamAudio {
                    audio_service_component_type,
                } if audio_service_component_type == dabplus::ASCTY_DAB_PLUS
                    && component.subchannel_id != 0xFF
                    && component.subchannel_bitrate > 0 =>
                {
                    Some((component.subchannel_id, component.subchannel_bitrate))
                }
                _ => None,
            }
        }) else {
            return Ok(());
        };

        self.stop_service().await?;
        self.tuner.open_subchannel(subchannel_id).await?;
        self.service = Some(ServiceSession {
            service_id,
            open_subchannel_id: subchannel_id,
            bitrate_kbits,
            decoder: DabPlusDecoder::new(bitrate_kbits),
            pad_decoder: pad::PadDecoder::new(),
            last_dls_text: String::new(),
            announcements: AnnouncementSwitcher::new(subchannel_id),
        });
        self.publish_snapshot(out);
        Ok(())
    }

    async fn handle_command(&mut self, command: Command, out: &Outputs) -> crate::Result<()> {
        match command {
            Command::Tune { frequency_khz, done } => {
                let result = self.do_tune(frequency_khz, out).await;
                if let Err(ref err) = result {
                    tracing::warn!(?err, frequency_khz, "device task: tune failed");
                }
                // `done` gets the real result; `handle_command` itself
                // always returns `Ok` here since the failure's already
                // been logged and/or handed to whoever's actually
                // waiting on it -- returning `Err` too would make `run`'s
                // own `Err` branch (below) log the exact same failure a
                // second time.
                if let Some(done) = done {
                    let _ = done.send(result);
                }
            }
            Command::StartService { service_id, done } => {
                let result = self.start_service(service_id, out).await;
                if let Err(ref err) = result {
                    tracing::warn!(?err, service_id, "device task: start_service failed");
                }
                if let Some(done) = done {
                    let _ = done.send(result);
                }
            }
            Command::StopService => {
                self.stop_service().await?;
                self.publish_snapshot(out);
            }
            Command::StartScan => {
                self.scanning = true;
                self.stop_service().await?;
                // A scan retunes the physical device to every channel in
                // turn, leaving it sitting on whichever one it last swept
                // once finished -- not necessarily (almost certainly not)
                // whatever was tuned before. Clearing `tuned` here (rather
                // than only when the sweep naturally completes) is what
                // stops the main loop's `poll_step` from ever running
                // against a stale `self.ensemble` on the wrong actual
                // frequency, for every path a scan can end on (finishes
                // sweeping, or an explicit `StopScan`) -- found by
                // inspection before this ever ran against real hardware:
                // without this, `self.tuned` could still read `true` from
                // before the scan started, and `poll_step` would resume
                // polling FIC on the scan's last channel while still
                // holding the *original* ensemble's stale data. The
                // caller must explicitly `Tune`/`StartService` again after
                // a scan to resume normal polling -- a deliberate
                // requirement, not an oversight.
                self.tuned = false;
                self.publish_snapshot(out);
            }
            Command::StopScan => {
                self.scanning = false;
                self.publish_snapshot(out);
            }
        }
        Ok(())
    }

    /// One channel's worth of a [`Command::StartScan`] sweep -- mirrors
    /// `examples/scan_and_listen.rs`'s `run_scan`, restructured as a
    /// single resumable step (called again next loop iteration) instead
    /// of a private function that owns the whole sweep loop, so
    /// `Command::StopScan` can be picked up between channels via the
    /// device task's normal non-racing `try_recv` check rather than
    /// needing its own cancellation plumbing.
    async fn scan_step(
        &mut self,
        cursor: &mut ScanCursor,
        out: &Outputs,
    ) -> crate::Result<()> {
        let channels = Tuner::channel_table_khz();
        loop {
            let Some(&khz) = channels.get(cursor.index) else {
                // Sweep finished.
                self.scanning = false;
                self.publish_snapshot(out);
                return Ok(());
            };
            let index = cursor.index;
            cursor.index += 1;
            if tables::DAB_N_FREQUENCIES_IDX.contains(&index) {
                continue;
            }

            let _ = out.scan_progress.send(ScanProgress {
                channel_index: index,
                channel_count: channels.len(),
                frequency_khz: khz,
                locked: false,
                failed: false,
                ensemble_label: None,
                service_count: 0,
            });

            let locked = self
                .tuner
                .tune_and_wait_for_lock(khz, std::time::Duration::from_secs(4), 1)
                .await?;
            if !locked {
                let _ = out.scan_progress.send(ScanProgress {
                    channel_index: index,
                    channel_count: channels.len(),
                    frequency_khz: khz,
                    locked: false,
                    failed: true,
                    ensemble_label: None,
                    service_count: 0,
                });
                return Ok(());
            }

            let mut ensemble = Ensemble::new();
            ensemble.set_frequency_khz(khz);
            // 20s, not `scan_and_listen`'s faster 10s default
            // (`--fic-timeout-secs`) -- `is_collect_finished()` already
            // exits this loop as soon as every playable service has its
            // label (FIG 1/1), so this only matters as a ceiling for
            // ensembles that don't reach that early. FIG 1/1 is broadcast
            // on its own cycle, separate from FIG 0/2 ("this service
            // exists"), and a multiplex with a dozen+ services can take
            // longer than 10s to cycle back around to a given one's
            // label -- confirmed live: a scan through the GUI left
            // several real, valid services with an empty label, not a
            // bug, just this window ending before their label FIG came
            // back around. User asked for fewer unnamed entries over a
            // faster scan.
            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_secs(20);
            while tokio::time::Instant::now() < deadline {
                let fic_bytes = self.tuner.poll_fic().await?;
                if !fic_bytes.is_empty() {
                    let events = fic::parse_fic(&fic_bytes);
                    ensemble.apply(&events);
                    if ensemble.is_collect_finished() {
                        break;
                    }
                }
            }

            let ensemble_id_known = ensemble.ensemble_id() != 0xFFFF;
            let service_count = ensemble.services().count();
            let _ = out.scan_progress.send(ScanProgress {
                channel_index: index,
                channel_count: channels.len(),
                frequency_khz: khz,
                locked: true,
                failed: false,
                ensemble_label: ensemble_id_known.then(|| ensemble.ensemble_label().to_string()),
                service_count: if ensemble_id_known { service_count } else { 0 },
            });
            if ensemble_id_known {
                let _ = out.ensemble_update.send(ensemble_info(&ensemble));
            }
            return Ok(());
        }
    }

    /// One poll cycle while an ensemble is tuned: always polls FIC
    /// (keeping the ensemble DB fresh and driving any active service's
    /// [`AnnouncementSwitcher`]), and additionally polls MSC + emits
    /// decoded audio/DLS/slideshow when a service is selected. Mirrors
    /// the reference's `readData` (both FIC and MSC1 branches off one
    /// `INT_E_STATL` read per iteration), except this reads the
    /// interrupt-status register twice (once inside each `Tuner::poll_*`
    /// call) -- the same tradeoff `Tuner::poll_msc`'s own doc comment
    /// already accepts, for the same reason.
    async fn poll_step(&mut self, out: &Outputs) -> crate::Result<()> {
        let fic_bytes = self.tuner.poll_fic().await?;
        if !fic_bytes.is_empty() {
            let events = fic::parse_fic(&fic_bytes);
            self.ensemble.apply(&events);
            let _ = out.ensemble_update.send(ensemble_info(&self.ensemble));

            if let Some(session) = &mut self.service {
                if let Some((new_subchan, event)) = session
                    .announcements
                    .update(&self.ensemble, session.service_id)
                {
                    let _ = out.announcement.send(event);
                    if new_subchan != session.open_subchannel_id {
                        // Genuinely switch the open subchannel for the
                        // announcement's duration (or back to the primary
                        // programme once it ends) -- this chip can only
                        // decode one subchannel at a time
                        // (`MAXIMUM_CONCURRENT_SUBCHANNELS = 1`, see
                        // `announcements.rs`'s own doc comment), so this
                        // really does interrupt the audio rather than
                        // duck under it. Bitrate carries over unchanged
                        // (announcement subchannels are still DAB+ audio
                        // components with their own signalled bitrate in
                        // principle, but `AnnouncementSwitcher` doesn't
                        // currently look it up -- reusing the primary
                        // programme's bitrate is the closest available
                        // approximation; flagging rather than silently
                        // assuming it's exactly right).
                        self.tuner.close_subchannel().await?;
                        self.tuner.open_subchannel(new_subchan).await?;
                        session.open_subchannel_id = new_subchan;
                        session.decoder = DabPlusDecoder::new(session.bitrate_kbits);
                        session.pad_decoder = pad::PadDecoder::new();
                        session.last_dls_text.clear();
                    }
                }
            }
        }

        if let Some(session) = &mut self.service {
            let msc_bytes = self.tuner.poll_msc().await?;
            if !msc_bytes.is_empty() {
                for output in session.decoder.push_msc_data(&msc_bytes) {
                    let _ = out.audio.send(AudioFrame {
                        service_id: session.service_id,
                        access_units: output.access_units,
                        access_unit_lengths: output.access_unit_lengths,
                        channels: output.channels,
                        sample_rate: output.sample_rate,
                        sbr_used: output.sbr_used,
                        ps_used: output.ps_used,
                    });
                    for pad_bytes in &output.pad_units {
                        let pad_out = session.pad_decoder.pad_data_input(pad_bytes);
                        if let Some(update) = pad_out.dls_update {
                            if update.text != session.last_dls_text {
                                session.last_dls_text = update.text.clone();
                                let _ = out.dls_update.send(DlsEvent {
                                    service_id: session.service_id,
                                    update,
                                });
                            }
                        }
                        for update in pad_out.slideshow_updates {
                            let _ = out.slideshow_update.send(SlideshowEvent {
                                service_id: session.service_id,
                                update,
                            });
                        }
                    }
                }
            }
        }

        let quality = self.tuner.signal_quality().await?;
        self.last_signal_quality = Some(quality);
        let _ = out.lock_status.send(quality);
        self.publish_snapshot(out);
        Ok(())
    }
}

/// Just enough scan-sweep state to make [`TaskState::scan_step`]
/// resumable one channel at a time across loop iterations.
struct ScanCursor {
    index: usize,
}

async fn run(tuner: Tuner, mut cmd_rx: mpsc::Receiver<Command>, out: std::sync::Arc<Outputs>) {
    let mut state = TaskState::new(tuner);
    let mut scan_cursor = ScanCursor { index: 0 };

    loop {
        // Non-blocking command check between steps -- never mid a USB
        // transaction, see this module's own doc comment on why.
        match cmd_rx.try_recv() {
            Ok(command) => {
                if let Err(err) = state.handle_command(command, &out).await {
                    tracing::warn!(?err, "device task: command failed");
                }
                if !state.scanning {
                    scan_cursor.index = 0;
                }
                continue;
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => return,
        }

        let step_result = if state.scanning {
            state.scan_step(&mut scan_cursor, &out).await
        } else if state.tuned {
            state.poll_step(&out).await
        } else {
            // Nothing to do until a command arrives -- block on it
            // directly (the only future here, so no cancel-safety
            // concern) instead of busy-looping.
            match cmd_rx.recv().await {
                Some(command) => state.handle_command(command, &out).await,
                None => return,
            }
        };

        if let Err(Error::DeviceDisconnected) = step_result {
            tracing::warn!("device task: device disconnected, attempting reconnect");
            if let Err(err) = state
                .tuner
                .reconnect(std::time::Duration::from_secs(2))
                .await
            {
                tracing::warn!(?err, "device task: reconnect failed, retrying next loop");
            }
        } else if let Err(err) = step_result {
            tracing::warn!(?err, "device task: step failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fic::fig::DabLabel;
    use crate::fic::{fig_00, fig_01, FigEvent};
    use crate::registered_tables::CharacterSet;

    fn label(text: &str) -> DabLabel {
        DabLabel {
            text: text.to_string(),
            short_text: text.to_string(),
        }
    }

    /// One audio service, `service_component_type` controlling whether
    /// it's DAB+ (`dabplus::ASCTY_DAB_PLUS`) or something else (e.g.
    /// classic MPEG audio, out of this crate's scope per `dabplus.rs`).
    fn one_audio_service(service_component_type: u8) -> Ensemble {
        let mut ensemble = Ensemble::new();
        ensemble.apply(&[
            FigEvent::Fig0000(fig_00::Fig0000 {
                ensemble_id: 0x1234,
                change_flag: fig_00::ChangeFlag::NoChange,
                is_alarm_supported: false,
                cif_count_high: 0,
                cif_count_low: 0,
                occurrence_change: 0,
            }),
            FigEvent::Fig0002(fig_00::ServiceDescription {
                service_id: 0x1001,
                ca_id: 0,
                num_service_components: 1,
                is_programme_service: true,
                service_components: vec![fig_00::ServiceComponentDescription {
                    transport_mode_id: fig_00::Tmid::MscStreamAudio,
                    sub_channel_id: 5,
                    is_primary: true,
                    is_ca_applied: false,
                    service_component_type,
                    service_component_id: 0xFFFF,
                }],
            }),
            FigEvent::Fig0001(fig_00::BasicSubchannelOrganization {
                is_programme_service: true,
                sub_channel_id: 5,
                start_address: 100,
                sub_channel_size: 72,
                sub_channel_bitrate: 128,
                convolutional_coding_rate: "3/4",
                protection_level_type: fig_00::ProtectionLevel::Level3A,
                protection_level: 2,
                protection_type: fig_00::ProtectionType::Uep,
                uep_table_index: 10,
            }),
            FigEvent::Fig0100(fig_01::Fig0100 {
                ensemble_id: 0x1234,
                label: label("My Ensemble"),
                charset: CharacterSet::EbuLatin,
            }),
            FigEvent::Fig0101(fig_01::Fig0101 {
                programme_service_id: 0x1001,
                label: label("Test FM"),
                charset: CharacterSet::EbuLatin,
            }),
        ]);
        ensemble
    }

    #[test]
    fn service_info_flags_dab_plus_audio_with_resolved_subchannel() {
        let ensemble = one_audio_service(dabplus::ASCTY_DAB_PLUS);
        let info = ensemble_info(&ensemble);
        assert_eq!(info.ensemble_id, 0x1234);
        assert_eq!(info.label, "My Ensemble");
        assert_eq!(info.services.len(), 1);
        let service = &info.services[0];
        assert_eq!(service.service_id, 0x1001);
        assert_eq!(service.label, "Test FM");
        assert!(
            service.has_dab_plus_audio,
            "ASCTY_DAB_PLUS with a resolved subchannel/bitrate should be playable"
        );
    }

    #[test]
    fn service_info_does_not_flag_non_dab_plus_audio() {
        // Classic MPEG audio (ASCTY 0x00) -- out of `dabplus.rs`'s scope,
        // so never "playable" through `Command::StartService` even though
        // it's a perfectly normal, fully-resolved audio component.
        let ensemble = one_audio_service(0x00);
        let info = ensemble_info(&ensemble);
        assert!(!info.services[0].has_dab_plus_audio);
    }

    #[test]
    fn service_lookup_matches_what_start_service_needs() {
        // `TaskState::start_service` looks up a service's DAB+ audio
        // component the same way `service_info` does -- exercised here
        // indirectly by confirming `Ensemble::service`/`component` return
        // exactly what `one_audio_service` set up, since `start_service`
        // itself needs a live `Tuner` (real hardware) to test end-to-end.
        let ensemble = one_audio_service(dabplus::ASCTY_DAB_PLUS);
        let service = ensemble.service(0x1001).expect("service exists");
        let key = service.component_keys[0];
        let component = ensemble.component(key).expect("component exists");
        assert_eq!(component.subchannel_id, 5);
        assert_eq!(component.subchannel_bitrate, 128);
    }

    #[test]
    fn snapshot_default_is_untuned_and_not_scanning() {
        let snapshot = Snapshot::default();
        assert!(snapshot.ensemble.is_none());
        assert!(snapshot.lock_status.is_none());
        assert!(snapshot.current_service_id.is_none());
        assert!(!snapshot.is_scanning);
    }
}
