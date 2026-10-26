pub mod config;
pub mod control_channel;
pub mod service_discovery;
// Renaming this to avoid the module_inception lint would ripple through every
// `stack::session::session::Session` import in the codebase; not worth it here.
#[allow(clippy::module_inception)]
pub mod session;
