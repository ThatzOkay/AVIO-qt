//! `#[tauri::command]` async fns -- the JS-callable surface, per
//! `RUST_PORT_PROMPT.md`'s Tauri plugin section's command list. Each one
//! here needs a matching `allow-<kebab-case-name>` entry in
//! `permissions/default.toml` and a matching name in `build.rs`'s
//! `COMMANDS` -- Tauri v2 denies any command not explicitly permitted.
//!
//! Every command/helper that takes an `AppHandle` is generic over
//! `R: tauri::Runtime` (`AppHandle<R>`, not the bare, runtime-defaulted
//! `AppHandle`) -- found the hard way: `#[tauri::command]`'s generated
//! wrapper is itself generic over an arbitrary `R`, and `AppHandle`
//! (which defaults its own type parameter to the concrete `Wry` runtime)
//! doesn't unify with that arbitrary `R` unless the command function
//! shares the same generic parameter. Using the bare, non-generic form
//! compiled fine in isolation but failed at the `generate_handler!` call
//! site in `lib.rs` with "the trait bound `AppHandle: CommandArg<'_, R>`
//! is not satisfied" -- real -- caught by actually building this crate,
//! not by review.

use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_store::StoreExt;
use tokio::sync::oneshot;

use wradio_c100_core::announcements::AnnouncementEvent;
use wradio_c100_core::device::{self, Command, DeviceHandle, EnsembleInfo, Snapshot};
use wradio_c100_core::store::{Preset, StationRecord, StationRef};
use wradio_c100_core::Tuner;

use crate::error::{PluginError, Result};
use crate::state::DabState;

/// Filename tauri-plugin-store persists the station DB/presets under
/// (resolved by the plugin against the app's own data directory) --
/// distinct from `wradio-c100-core`'s own `store.rs`, which is a plain
/// `serde_json` file used only by the headless `scan_and_listen` example;
/// this plugin uses `tauri-plugin-store` instead per
/// `RUST_PORT_PROMPT.md`'s explicit instruction ("Since this is already a
/// Tauri plugin, use tauri-plugin-store... rather than hand-rolling file
/// I/O"), reusing the same `StationRecord`/`Preset`/`StationRef` DTOs
/// (already `Serialize`/`Deserialize` for exactly this reuse, per that
/// module's own doc comment).
const STORE_PATH: &str = "wradio-c100-stations.json";
const PRESETS_KEY: &str = "presets";
const STATIONS_KEY: &str = "stations";

/// Whether a wRadio C100 dongle (VID:PID 16c0:05dc) is currently
/// enumerable over USB -- a cheap, non-exclusive check (unlike
/// `open_device`, this never claims the interface). Shared between the
/// `list_devices` command (a one-shot poll, e.g. for the initial paint)
/// and `crate::hotplug`'s live watcher.
pub(crate) async fn device_present() -> bool {
    // `nusb::list_devices()` returns a future of an iterator (not a bare
    // iterator) in the pinned `nusb` version -- same shape
    // `wradio-c100-core/src/usb.rs`'s own `UsbDevice::open_first` already
    // awaits, matched here rather than guessed.
    nusb::list_devices()
        .await
        .map(|mut devices| {
            devices.any(|d| {
                d.vendor_id() == wradio_c100_core::usb::VENDOR_ID
                    && d.product_id() == wradio_c100_core::usb::PRODUCT_ID
            })
        })
        .unwrap_or(false)
}

#[tauri::command]
pub async fn list_devices() -> Result<bool> {
    Ok(device_present().await)
}

/// Opens the device, brings up the tuner chip, and spawns the core's
/// device task (`wradio_c100_core::device::spawn`) -- idempotent: a
/// second call while a device is already open is a no-op rather than an
/// error (matches [`DabState`]'s own doc comment on why "not open yet"
/// has to be a normal, retryable state).
///
/// Also spawns this plugin's event-forwarding tasks (one per output
/// channel, per `RUST_PORT_PROMPT.md`'s Tauri plugin section) here rather
/// than in `Builder::setup`, since the device (and therefore the
/// `DeviceHandle` those tasks read from) doesn't exist until this command
/// actually succeeds -- `setup` only runs once at app startup, before the
/// frontend has had a chance to call this at all.
///
/// Called two ways: directly as the `#[tauri::command]` the frontend's
/// own "Open device" button invokes, *and* by `crate::hotplug`'s watcher
/// auto-opening on a fresh connect -- emitting `device-opened` here
/// (rather than in either caller separately) covers both for free, which
/// matters specifically for the hotplug path: it never goes through the
/// frontend's own `openDevice()` JS call at all, so without a live event
/// here the frontend would have no way to learn the device it never
/// asked to open is actually open now.
#[tauri::command]
pub async fn open_device<R: Runtime>(app: AppHandle<R>, state: State<'_, DabState>) -> Result<()> {
    let mut guard = state.device.lock().await;
    if guard.is_some() {
        return Ok(());
    }
    let mut tuner = Tuner::open().await?;
    tuner.initialize().await?;
    let handle = device::spawn(tuner);
    let _ = app.emit("wradio-c100://device-opened", true);
    spawn_event_forwarders(app, &handle);
    *state.playback.lock().await = Some(crate::playback::spawn(&handle));
    *guard = Some(handle);
    Ok(())
}

async fn device_handle<'a>(
    state: &'a State<'_, DabState>,
) -> Result<tokio::sync::MutexGuard<'a, Option<DeviceHandle>>> {
    let guard = state.device.lock().await;
    if guard.is_none() {
        return Err(PluginError::new(
            "device not open -- call open_device first",
        ));
    }
    Ok(guard)
}

async fn send_command(state: &State<'_, DabState>, command: Command) -> Result<()> {
    let guard = device_handle(state).await?;
    guard
        .as_ref()
        .expect("checked Some above")
        .send(command)
        .await?;
    Ok(())
}

/// Like [`send_command`], but for `Tune`/`StartService`: actually waits
/// for the device task to finish processing the command (tuned + locked,
/// or the service switch applied) instead of resolving as soon as it's
/// enqueued. `DeviceHandle::send` resolving only ever meant "the task is
/// alive to receive it" (that type's own doc comment) -- for a `Tune`
/// that can legitimately take several real seconds
/// (`tune_and_wait_for_lock`'s retry budget), the frontend's "still
/// tuning, don't let the user pile on more clicks" state needs the real
/// completion, not the enqueue ack; confirmed live that the two are
/// several seconds apart, not close enough to treat as the same moment.
async fn send_command_and_wait(
    state: &State<'_, DabState>,
    make_command: impl FnOnce(oneshot::Sender<wradio_c100_core::Result<()>>) -> Command,
) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    {
        let guard = device_handle(state).await?;
        guard
            .as_ref()
            .expect("checked Some above")
            .send(make_command(tx))
            .await?;
        // `guard` drops at the end of this block -- held only long enough
        // to enqueue the command, not for however long the device task
        // actually takes to process it. Holding it across the `rx.await`
        // below instead would block every other command on this same
        // `state.device` mutex (volume, snapshot, even another tune) for
        // the whole retune, which the previous fire-and-forget version
        // never did and this shouldn't newly start doing.
    }
    match rx.await {
        Ok(result) => result.map_err(Into::into),
        Err(_) => Err(PluginError::new(
            "device task stopped before finishing this command",
        )),
    }
}

#[tauri::command]
pub async fn start_scan(state: State<'_, DabState>) -> Result<()> {
    send_command(&state, Command::StartScan).await
}

#[tauri::command]
pub async fn stop_scan(state: State<'_, DabState>) -> Result<()> {
    send_command(&state, Command::StopScan).await
}

#[tauri::command]
pub async fn tune(state: State<'_, DabState>, frequency_khz: u32) -> Result<()> {
    send_command_and_wait(&state, |done| Command::Tune {
        frequency_khz,
        done: Some(done),
    })
    .await
}

#[tauri::command]
pub async fn start_service(state: State<'_, DabState>, service_id: u32) -> Result<()> {
    send_command_and_wait(&state, |done| Command::StartService {
        service_id,
        done: Some(done),
    })
    .await
}

#[tauri::command]
pub async fn stop_service(state: State<'_, DabState>) -> Result<()> {
    send_command(&state, Command::StopService).await
}

/// `volume` is clamped to `0.0..=1.0` (see `PlaybackHandle::set_volume`) --
/// out-of-range input is silently clamped rather than rejected, matching
/// how most native volume APIs behave.
#[tauri::command]
pub async fn set_volume(state: State<'_, DabState>, volume: f32) -> Result<()> {
    let guard = state.playback.lock().await;
    let Some(playback) = guard.as_ref() else {
        return Err(PluginError::new("device not open -- call open_device first"));
    };
    playback.set_volume(volume);
    Ok(())
}

#[tauri::command]
pub async fn mute(state: State<'_, DabState>, muted: bool) -> Result<()> {
    let guard = state.playback.lock().await;
    let Some(playback) = guard.as_ref() else {
        return Err(PluginError::new("device not open -- call open_device first"));
    };
    playback.set_muted(muted);
    Ok(())
}

/// The current ensemble/service DB + lock status + selected service,
/// mirroring [`device::DeviceHandle::snapshot`]'s latest `watch` value --
/// for the frontend's initial render (before the first push event
/// arrives) or a manual refresh, rather than waiting on events alone.
#[tauri::command]
pub async fn get_snapshot(state: State<'_, DabState>) -> Result<Snapshot> {
    let guard = device_handle(&state).await?;
    Ok(guard.as_ref().expect("checked Some above").snapshot())
}

fn load_presets<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<Preset>> {
    let store = app.store(STORE_PATH)?;
    match store.get(PRESETS_KEY) {
        // Same reasoning as `load_stations`'s own `unwrap_or_default` --
        // applied here too for consistency even though `Preset`'s shape
        // hasn't itself changed yet, so this class of bug can't
        // reappear the next time it does.
        Some(value) => Ok(serde_json::from_value(value).unwrap_or_default()),
        None => Ok(Vec::new()),
    }
}

fn save_presets<R: Runtime>(app: &AppHandle<R>, presets: &[Preset]) -> Result<()> {
    let store = app.store(STORE_PATH)?;
    store.set(PRESETS_KEY, serde_json::to_value(presets)?);
    store.save()?;
    Ok(())
}

#[tauri::command]
pub fn list_presets<R: Runtime>(app: AppHandle<R>) -> Result<Vec<Preset>> {
    load_presets(&app)
}

/// Assigns `station` to `slot`, overwriting whatever was there before
/// (matches `store.rs`'s own `Store::set_preset` semantics: "re-assigning
/// a slot overwrites rather than duplicating it").
#[tauri::command]
pub fn set_preset<R: Runtime>(app: AppHandle<R>, slot: u32, station: StationRef) -> Result<()> {
    let mut presets = load_presets(&app)?;
    presets.retain(|p| p.slot != slot);
    presets.push(Preset { slot, station });
    save_presets(&app, &presets)
}

#[tauri::command]
pub fn clear_preset<R: Runtime>(app: AppHandle<R>, slot: u32) -> Result<()> {
    let mut presets = load_presets(&app)?;
    presets.retain(|p| p.slot != slot);
    save_presets(&app, &presets)
}

fn load_stations<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<StationRecord>> {
    let store = app.store(STORE_PATH)?;
    match store.get(STATIONS_KEY) {
        // A deserialize failure here means previously-persisted data
        // doesn't match `StationRecord`'s current shape (e.g. this exact
        // scenario, hit for real: adding `has_dab_plus_audio` made every
        // record a prior scan had already saved fail to deserialize).
        // Treated as empty rather than propagated as an error -- matches
        // `wradio_c100_core::store::Store::open`'s own established
        // precedent for this exact kind of data ("A missing or corrupt
        // file starts an empty store rather than erroring... this is a
        // scan-result cache, not critical data, and any of it is
        // recoverable by rescanning"). Without this, every command that
        // touches the station DB (`list_stations`, and
        // `upsert_stations_from_ensemble` on every single ensemble
        // update) would hard-fail after any future schema change until
        // the user manually deleted the store file.
        Some(value) => Ok(serde_json::from_value(value).unwrap_or_default()),
        None => Ok(Vec::new()),
    }
}

fn save_stations<R: Runtime>(app: &AppHandle<R>, stations: &[StationRecord]) -> Result<()> {
    let store = app.store(STORE_PATH)?;
    store.set(STATIONS_KEY, serde_json::to_value(stations)?);
    store.save()?;
    Ok(())
}

/// The persisted station database -- every service seen so far across
/// every scan/tune, each with every frequency it's been found on. This is
/// what resolves a preset's bare [`StationRef`] (just an ensemble+service
/// id, no frequency) back into something actually playable, and lets the
/// frontend show "known stations" without having to be actively tuned to
/// their ensemble right now.
#[tauri::command]
pub fn list_stations<R: Runtime>(app: AppHandle<R>) -> Result<Vec<StationRecord>> {
    load_stations(&app)
}

/// Upserts every service in `ensemble` into the persisted station DB --
/// mirrors `wradio_c100_core::store::Store::upsert_station`'s exact
/// semantics (same 0xFFFF-sentinel guard, same "refresh label/PTY, add
/// the frequency as a candidate if new, never delete" behavior) since
/// this plugin doesn't reuse that type directly (it's the *headless*
/// `scan_and_listen` example's own plain-file store, not this plugin's
/// `tauri-plugin-store`-backed one) -- called from the `ensemble_update`
/// event forwarder below on every FIG 0/0+0/2 collection cycle, whether
/// that arrived from `Command::Tune` or a `Command::StartScan` sweep, so
/// presets/`list_stations` stay populated the same way real listening
/// naturally discovers stations, not only during an explicit scan.
fn upsert_stations_from_ensemble<R: Runtime>(app: &AppHandle<R>, ensemble: &EnsembleInfo) {
    // A 0xFFFF ensemble id means FIG 0/0 hasn't resolved yet within
    // whatever collection window produced this `EnsembleInfo` -- matches
    // `Ensemble::new()`'s own "not yet known" sentinel. Persisting under
    // it would create permanent duplicate entries once the real id is
    // later learned (the exact real bug `Store::upsert_station` itself
    // was fixed for, Phase 7 -- see `PORT_CHECKLIST.md`), so skip
    // entirely rather than guess.
    if ensemble.ensemble_id == 0xFFFF {
        return;
    }
    let mut stations = match load_stations(app) {
        Ok(stations) => stations,
        Err(err) => {
            tracing::warn!(%err, "failed to load station DB before upserting, skipping this update");
            return;
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    for service in &ensemble.services {
        let station_ref = StationRef {
            ensemble_id: ensemble.ensemble_id,
            service_id: service.service_id,
        };
        if let Some(existing) = stations.iter_mut().find(|s| s.station == station_ref) {
            if !service.label.is_empty() {
                existing.label = service.label.clone();
            }
            existing.programme_type_code = service.programme_type_code;
            existing.has_dab_plus_audio = service.has_dab_plus_audio;
            if !existing.frequencies_khz.contains(&ensemble.frequency_khz) {
                existing.frequencies_khz.push(ensemble.frequency_khz);
            }
            existing.last_seen_unix = now;
        } else {
            stations.push(StationRecord {
                station: station_ref,
                label: service.label.clone(),
                programme_type_code: service.programme_type_code,
                has_dab_plus_audio: service.has_dab_plus_audio,
                frequencies_khz: vec![ensemble.frequency_khz],
                last_seen_unix: now,
            });
        }
    }
    if let Err(err) = save_stations(app, &stations) {
        tracing::warn!(%err, "failed to save station DB after upserting");
    }
}

/// Subscribes to every [`DeviceHandle`] output channel and forwards each
/// event to the frontend via `app_handle.emit`, one task per channel
/// (`RUST_PORT_PROMPT.md`'s Tauri plugin section: "Events: forward the
/// core's output channels to the frontend... one forwarding task per
/// channel"). Each loop keeps running across a `RecvError::Lagged` (a
/// slow/momentarily-detached frontend listener missed some events, but
/// forwarding should continue with whatever arrives next) and only stops
/// on `RecvError::Closed` (the `DeviceHandle` -- and therefore the
/// device task itself -- is gone).
fn spawn_event_forwarders<R: Runtime>(app: AppHandle<R>, device: &DeviceHandle) {
    forward(
        app.clone(),
        device.subscribe_scan_progress(),
        "wradio-c100://scan-progress",
    );
    let mut ensemble_rx = device.subscribe_ensemble_update();
    let ensemble_app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match ensemble_rx.recv().await {
                Ok(ensemble) => {
                    upsert_stations_from_ensemble(&ensemble_app, &ensemble);
                    let _ = ensemble_app.emit("wradio-c100://ensemble-update", ensemble);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, "ensemble-update forwarder lagged, some events dropped");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    forward(
        app.clone(),
        device.subscribe_lock_status(),
        "wradio-c100://lock-status",
    );
    forward(
        app.clone(),
        device.subscribe_dls_update(),
        "wradio-c100://dls-update",
    );
    forward(
        app.clone(),
        device.subscribe_slideshow_update(),
        "wradio-c100://slideshow-update",
    );

    let mut announcement_rx = device.subscribe_announcement();
    let announcement_app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match announcement_rx.recv().await {
                Ok(event) => {
                    let name = match &event {
                        AnnouncementEvent::Started { .. } => "wradio-c100://announcement-start",
                        AnnouncementEvent::Ended { .. } => "wradio-c100://announcement-end",
                    };
                    let _ = announcement_app.emit(name, event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, "announcement event forwarder lagged, some events dropped");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // `Snapshot` (`isScanning`/`currentServiceId`/etc.) is a `watch`
    // channel, not one of the `broadcast` ones `forward` handles -- and
    // unlike those, nothing was pushing it to the frontend at all until
    // this was added: the frontend only ever pulled it once, right after
    // `open_device` succeeds (`get_snapshot`), so `startScan`/
    // `startService`/etc. changing the *actual* device-task state never
    // reached the UI on their own. Reported live: clicking "Scan for
    // stations" genuinely started a scan (confirmed via `scan-progress`
    // events, which *were* already wired), but the button never flipped
    // to "Stop scan" and the progress line never appeared, because both
    // read `isScanning` off a `Snapshot` that was frozen at whatever it
    // was the moment the device was opened.
    let mut snapshot_rx = device.watch_snapshot();
    let snapshot_app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Emit the value already present before the first change too --
        // `watch::Receiver::changed()` alone only resolves on *future*
        // updates, so without this the frontend would still be stuck on
        // whatever `get_snapshot` returned until the *next* state change.
        let _ = snapshot_app.emit(
            "wradio-c100://snapshot",
            snapshot_rx.borrow_and_update().clone(),
        );
        while snapshot_rx.changed().await.is_ok() {
            let _ = snapshot_app.emit(
                "wradio-c100://snapshot",
                snapshot_rx.borrow_and_update().clone(),
            );
        }
    });
}

fn forward<T, R: Runtime>(
    app: AppHandle<R>,
    mut rx: tokio::sync::broadcast::Receiver<T>,
    event_name: &'static str,
) where
    T: Clone + serde::Serialize + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(payload) => {
                    let _ = app.emit(event_name, payload);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, event_name, "event forwarder lagged, some events dropped");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
