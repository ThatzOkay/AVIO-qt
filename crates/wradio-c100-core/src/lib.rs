//! Async Rust core for the wRadio C100 / Raontech-based USB DAB+ dongle
//! (VID:PID 16c0:05dc).
//!
//! This crate has no Tauri dependency -- it's a plain, reusable DAB+
//! stack: USB transport, tuner bring-up/tuning, FIC parsing into an
//! ensemble/service database, MSC subchannel demux and DAB+ superframe
//! extraction, and DLS/slideshow metadata decoding. See the crate's
//! `examples/` for headless usage.
//!
//! Current status: USB transport, register protocol, and tuner bring-up/
//! tuning are implemented (`usb`, `hal`, `tables`, `tuner`), as is full FIC
//! parsing into an ensemble/service database (`fic`, `registered_tables`,
//! `ensemble`, `service`), the DAB+ MSC pipeline (`rs`, `dabplus`):
//! subchannel open/close, Firecode frame sync, Reed-Solomon correction, and
//! AU splitting, producing raw AAC access units + audio params, and the
//! full metadata stack (`pad`, `dls`, `mot`, `slideshow`): X-PAD framing,
//! Dynamic Label/DL+ decode (the DAB "RDS RadioText/RT+" equivalent), and
//! MOT slideshow decode (station logo/album art) -- all confirmed against
//! real over-the-air broadcast data. `tuner.rs` also has auto-reset-on-
//! stuck-tuning (`tune_and_wait_for_lock`) and USB disconnect/reconnect
//! (`reconnect`, backed by `Error::DeviceDisconnected`) -- auto-mute needs
//! a native playback layer to actually own and isn't built yet, and none
//! of this is wired onto a broadcast/watch channel yet (deferred to the
//! device-task architecture a Tauri plugin needs). `store`/
//! `service_following`/`announcements` add a persisted station database +
//! presets, Synthetic Service Following, and FIG 0/18/0/19-driven
//! announcement switching -- new functionality with no reference C++ to
//! port (see `store.rs`'s own doc comment for why); auto-tune on startup
//! is confirmed against real hardware, service following/announcements
//! are unit-tested but not yet wired into the example (need a
//! multi-frequency scan / playback pipeline respectively, both later
//! phases). `examples/scan_and_listen.rs` now wires all of the above
//! together: a full-channel-table `--scan`, `--service-name` selection,
//! periodic signal bars, and reconnect-on-disconnect -- all confirmed
//! against real hardware except reconnect itself (needs a physical
//! unplug/replug to verify live). The Tauri plugin + example app land in
//! later stages.

pub mod adts;
pub mod announcements;
pub mod dabplus;
pub mod device;
pub mod dls;
pub mod ensemble;
pub mod error;
pub mod fic;
pub mod hal;
pub mod mot;
pub mod pad;
pub mod registered_tables;
pub mod rs;
pub mod service;
pub mod service_following;
pub mod slideshow;
pub mod store;
pub mod tables;
pub mod tuner;
pub mod usb;

pub use error::{Error, Result};
pub use tuner::Tuner;
