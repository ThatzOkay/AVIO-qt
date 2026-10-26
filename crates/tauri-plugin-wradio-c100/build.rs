//! Generates this plugin's permission ACL types from `permissions/*.toml`
//! at build time -- the standard `cargo create-tauri-plugin` scaffold
//! shape (`RUST_PORT_PROMPT.md`'s Tauri plugin section: "follow its
//! conventional plugin crate shape rather than inventing your own").
//! `COMMANDS` must list every `#[tauri::command]` in `src/commands.rs` --
//! Tauri v2 denies any command not explicitly permitted, and permission
//! identifiers are auto-generated from these names (snake_case ->
//! `allow-kebab-case`, e.g. `start_scan` -> `allow-start-scan`), matching
//! `permissions/default.toml`.

const COMMANDS: &[&str] = &[
    "list_devices",
    "open_device",
    "start_scan",
    "stop_scan",
    "tune",
    "start_service",
    "stop_service",
    "get_snapshot",
    "list_presets",
    "set_preset",
    "clear_preset",
    "list_stations",
    "set_volume",
    "mute",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
