//! Thin Tauri v2 plugin wrapping [`wradio_c100_core`] -- see this repo's
//! `RUST_PORT_PROMPT.md`, "Tauri plugin" section, for the full spec this
//! follows. All hardware/DAB logic lives in the core crate (zero Tauri
//! dependency there, so it stays reusable outside Tauri too, e.g.
//! `wradio-c100-core/examples/scan_and_listen.rs`); this crate is
//! commands + permission declarations + event forwarding only.
//!
//! Follows the conventional `cargo create-tauri-plugin` shape rather than
//! inventing a new one: `commands.rs` (the `#[tauri::command]` surface),
//! `models.rs` (re-exported DTOs), `state.rs` (the managed
//! `tauri::State`), `error.rs` (the serializable error type every
//! command's `Err` needs), `build.rs` + `permissions/default.toml`
//! (Tauri v2's command allow-list, generated at build time from the same
//! command list `build.rs` declares).

mod commands;
mod error;
mod hotplug;
mod models;
mod playback;
mod state;

use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

pub use error::PluginError;
pub use models::*;
pub use state::DabState;

/// Builds the plugin. Call `.plugin(tauri_plugin_wradio_c100::init())` on
/// the app's own `tauri::Builder` (in `examples/tauri-app/src-tauri`) --
/// alongside `.plugin(tauri_plugin_store::Builder::default().build())`,
/// since `commands::list_presets`/`set_preset`/`clear_preset` need that
/// plugin registered too (see `commands.rs`'s own doc comment on why
/// presets go through `tauri-plugin-store` rather than this crate
/// bundling its own persistence).
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    // `Builder::new(...)` alone leaves the runtime type parameter
    // unconstrained here (nothing in `.setup`'s closure or
    // `generate_handler!` pins it to this fn's own generic `R`), and it
    // silently defaults to the concrete `Wry` runtime instead -- caught
    // by the compiler (`expected TauriPlugin<R>, found
    // TauriPlugin<Wry<EventLoopMessage>, _>`) rather than a runtime
    // surprise; `::<R>` forces it to stay generic.
    Builder::<R>::new("wradio-c100")
        .invoke_handler(tauri::generate_handler![
            commands::list_devices,
            commands::open_device,
            commands::start_scan,
            commands::stop_scan,
            commands::tune,
            commands::start_service,
            commands::stop_service,
            commands::get_snapshot,
            commands::list_presets,
            commands::set_preset,
            commands::clear_preset,
            commands::list_stations,
            commands::set_volume,
            commands::mute,
        ])
        .setup(|app, _api| {
            // The device itself isn't opened *here*, unconditionally, at
            // startup -- see `DabState`'s own doc comment on why "not
            // present yet" has to be a normal, non-fatal state. It can
            // still end up open moments later without any user action:
            // `hotplug::spawn` below opens it itself the instant it sees
            // the dongle present, both on this initial check and on every
            // future reconnect -- must run after `manage()`, which is
            // what makes `AppHandle::state::<DabState>()` valid to call
            // from within it.
            app.manage(state::DabState::default());
            hotplug::spawn(app.clone());
            Ok(())
        })
        .build()
}
