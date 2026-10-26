//! Plugin-local error type.
//!
//! Tauri v2 requires every command's `Err` type to implement
//! `serde::Serialize` (`RUST_PORT_PROMPT.md`'s Tauri plugin section calls
//! this out explicitly as "a hard Tauri requirement") -- `Error` in
//! `wradio_c100_core::error` doesn't, and shouldn't: it's a `thiserror`
//! type with `#[from] nusb::Error`/`std::io::Error` variants, and adding
//! a Tauri-specific trait bound to a plain async core library the port
//! plan explicitly keeps Tauri-free would be the wrong place to pay for
//! this. Mapped into this serializable DTO here instead of bubbling the
//! core error up directly, matching that section's own instruction: "map
//! the core's `thiserror` type into a serializable DTO in `error.rs`,
//! don't just bubble up `anyhow`."

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginError {
    pub message: String,
}

impl PluginError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PluginError {}

impl From<wradio_c100_core::Error> for PluginError {
    fn from(err: wradio_c100_core::Error) -> Self {
        Self::new(err.to_string())
    }
}

impl From<tauri_plugin_store::Error> for PluginError {
    fn from(err: tauri_plugin_store::Error) -> Self {
        Self::new(err.to_string())
    }
}

impl From<serde_json::Error> for PluginError {
    fn from(err: serde_json::Error) -> Self {
        Self::new(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, PluginError>;
