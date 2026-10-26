// Thin typed wrappers over `invoke()`/`listen()` for this plugin's
// commands and events -- only what `src/commands.rs` actually implements
// (`RUST_PORT_PROMPT.md`'s Tauri plugin section: "thin typed wrappers
// only over what you actually implemented"). Field names are camelCase
// throughout, matching every Rust DTO's `#[serde(rename_all =
// "camelCase")]` (`wradio-c100-core/src/device.rs` and `store.rs`) --
// keep these interfaces in lock-step with those structs by hand; nothing
// generates this file.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

const PLUGIN = "wradio-c100";
const cmd = (name: string) => `plugin:${PLUGIN}|${name}`;

// ---- DTOs (mirror wradio-c100-core/src/device.rs and src/store.rs) ----

export interface ServiceInfo {
  serviceId: number;
  label: string;
  programmeTypeCode: number;
  hasDabPlusAudio: boolean;
}

export interface EnsembleInfo {
  ensembleId: number;
  label: string;
  ecc: number;
  frequencyKhz: number;
  services: ServiceInfo[];
}

export interface ScanProgress {
  channelIndex: number;
  channelCount: number;
  frequencyKhz: number;
  locked: boolean;
  ensembleLabel: string | null;
  serviceCount: number;
}

export interface LockStatus {
  ofdmLocked: boolean;
  fecLocked: boolean;
}

export interface SignalQuality {
  locked: boolean;
  signalBars: number | null;
  muted: boolean;
  deviceConnected: boolean;
}

export interface DlPlusTag {
  contentType: string;
  text: string;
}

export interface DlsUpdate {
  text: string;
  charset: string;
  itemToggle: boolean;
  itemRunning: boolean;
  plusTags: DlPlusTag[];
}

export interface DlsEvent {
  serviceId: number;
  update: DlsUpdate;
}

export interface SlideshowUpdate {
  isCategorized: boolean;
  contentName: string;
  categoryId: number;
  slideId: number;
  categoryTitle: string;
  clickThroughUrl: string;
  alternativeLocationUrl: string;
  mimeType: string;
  contentSubType: number;
  /** JSON array of bytes -- see `SlideshowUpdate`'s own Rust doc comment
   * on why this isn't base64. Build a data URL client-side, e.g.
   * `URL.createObjectURL(new Blob([new Uint8Array(update.slideshowData)], { type: update.mimeType }))`. */
  slideshowData: number[];
}

export interface SlideshowEvent {
  serviceId: number;
  update: SlideshowUpdate;
}

export type AnnouncementEvent =
  | { type: "started"; announcementType: string; subchannelId: number }
  | { type: "ended"; subchannelId: number };

export interface Snapshot {
  ensemble: EnsembleInfo | null;
  lockStatus: SignalQuality | null;
  currentServiceId: number | null;
  isScanning: boolean;
}

export interface StationRef {
  ensembleId: number;
  serviceId: number;
}

export interface Preset {
  slot: number;
  station: StationRef;
}

export interface StationRecord {
  station: StationRef;
  label: string;
  programmeTypeCode: number;
  /** Whether `startService`/`playStation` would actually play this -- false for TPEG and other data-only/non-DAB+ services. */
  hasDabPlusAudio: boolean;
  /** Candidate frequencies (kHz), index 0 tried first -- see `promote_frequency` in `wradio-c100-core/src/store.rs`. */
  frequenciesKhz: number[];
  lastSeenUnix: number;
}

// ---- Commands ----

export function listDevices(): Promise<boolean> {
  return invoke(cmd("list_devices"));
}

export function openDevice(): Promise<void> {
  return invoke(cmd("open_device"));
}

export function startScan(): Promise<void> {
  return invoke(cmd("start_scan"));
}

export function stopScan(): Promise<void> {
  return invoke(cmd("stop_scan"));
}

export function tune(frequencyKhz: number): Promise<void> {
  return invoke(cmd("tune"), { frequencyKhz });
}

export function startService(serviceId: number): Promise<void> {
  return invoke(cmd("start_service"), { serviceId });
}

export function stopService(): Promise<void> {
  return invoke(cmd("stop_service"));
}

export function getSnapshot(): Promise<Snapshot> {
  return invoke(cmd("get_snapshot"));
}

export function listPresets(): Promise<Preset[]> {
  return invoke(cmd("list_presets"));
}

export function setPreset(slot: number, station: StationRef): Promise<void> {
  return invoke(cmd("set_preset"), { slot, station });
}

export function clearPreset(slot: number): Promise<void> {
  return invoke(cmd("clear_preset"), { slot });
}

export function listStations(): Promise<StationRecord[]> {
  return invoke(cmd("list_stations"));
}

/** Clamped to 0.0-1.0 on the Rust side. */
export function setVolume(volume: number): Promise<void> {
  return invoke(cmd("set_volume"), { volume });
}

export function mute(muted: boolean): Promise<void> {
  return invoke(cmd("mute"), { muted });
}

// ---- Events ----

/** Fires once immediately with the current value, then again on every
 * change -- `isScanning`/`currentServiceId` (and everything else in
 * `Snapshot`) only ever update via this event, not by re-calling
 * `getSnapshot()` yourself after every command. */
export function onSnapshot(handler: (snapshot: Snapshot) => void): Promise<UnlistenFn> {
  return listen<Snapshot>(`${PLUGIN}://snapshot`, (e) => handler(e.payload));
}

export function onScanProgress(handler: (event: ScanProgress) => void): Promise<UnlistenFn> {
  return listen<ScanProgress>(`${PLUGIN}://scan-progress`, (e) => handler(e.payload));
}

export function onEnsembleUpdate(handler: (event: EnsembleInfo) => void): Promise<UnlistenFn> {
  return listen<EnsembleInfo>(`${PLUGIN}://ensemble-update`, (e) => handler(e.payload));
}

/** Live USB presence for the dongle (VID:PID 16c0:05dc) -- fires on every
 * connect/disconnect, backed by `hotplug.rs`'s watcher, not a poll. The
 * device also auto-opens itself on the backend the moment this goes
 * true (if nothing's already open), so this is mainly useful for driving
 * the "no device" empty state rather than for deciding whether to call
 * `openDevice()` yourself. */
export function onDevicePresence(handler: (present: boolean) => void): Promise<UnlistenFn> {
  return listen<boolean>(`${PLUGIN}://device-presence`, (e) => handler(e.payload));
}

/** Fires once the device is actually open -- whether from this webview's
 * own `openDevice()` call, or `hotplug.rs` auto-opening it before the
 * frontend ever asked. Listen for this instead of only trusting
 * `openDevice()`'s own promise to know when the app has moved past the
 * "no device" gate. */
export function onDeviceOpened(handler: () => void): Promise<UnlistenFn> {
  return listen<boolean>(`${PLUGIN}://device-opened`, () => handler());
}

export function onLockStatus(handler: (event: SignalQuality) => void): Promise<UnlistenFn> {
  return listen<SignalQuality>(`${PLUGIN}://lock-status`, (e) => handler(e.payload));
}

export function onDlsUpdate(handler: (event: DlsEvent) => void): Promise<UnlistenFn> {
  return listen<DlsEvent>(`${PLUGIN}://dls-update`, (e) => handler(e.payload));
}

export function onSlideshowUpdate(handler: (event: SlideshowEvent) => void): Promise<UnlistenFn> {
  return listen<SlideshowEvent>(`${PLUGIN}://slideshow-update`, (e) => handler(e.payload));
}

export function onAnnouncementStart(
  handler: (event: Extract<AnnouncementEvent, { type: "started" }>) => void,
): Promise<UnlistenFn> {
  return listen<Extract<AnnouncementEvent, { type: "started" }>>(`${PLUGIN}://announcement-start`, (e) =>
    handler(e.payload),
  );
}

export function onAnnouncementEnd(
  handler: (event: Extract<AnnouncementEvent, { type: "ended" }>) => void,
): Promise<UnlistenFn> {
  return listen<Extract<AnnouncementEvent, { type: "ended" }>>(`${PLUGIN}://announcement-end`, (e) =>
    handler(e.payload),
  );
}
