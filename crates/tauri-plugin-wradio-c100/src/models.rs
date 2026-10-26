//! DTOs for command results and event payloads.
//!
//! All of these are re-exports of types that already live in
//! `wradio_c100_core` -- `device.rs` there defines `EnsembleInfo`/
//! `ServiceInfo`/`ScanProgress`/`Snapshot`/`DlsEvent`/`SlideshowEvent` as
//! `Serialize` + `#[serde(rename_all = "camelCase")]` from the start
//! specifically so this plugin crate wouldn't need to duplicate them --
//! see `RUST_PORT_PROMPT.md`'s core-crate suggested-dependencies note:
//! "serde+serde derive on the DTO-ish public types... since the plugin
//! crate will need to serialize them for IPC -- no reason to duplicate
//! the structs." This module exists (rather than `commands.rs` importing
//! straight from `wradio_c100_core`) to match the suggested plugin crate
//! layout's `models.rs` file, and to be the one place a genuinely
//! plugin-specific DTO would live if one's ever needed.

pub use wradio_c100_core::announcements::AnnouncementEvent;
pub use wradio_c100_core::device::{
    AudioFrame, Command, DlsEvent, EnsembleInfo, ScanProgress, ServiceInfo, SlideshowEvent,
    Snapshot,
};
pub use wradio_c100_core::store::{Preset, StationRecord, StationRef};
