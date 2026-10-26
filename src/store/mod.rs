use std::sync::Mutex;

use qtbridge::QmlMethodInvoker;

pub mod radio_store;
pub mod settings_store;
pub mod status_store;
pub mod udev_store;

// Qt creates the store singletons itself, so Rust code can't hold them directly. Each store hands
// its thread-safe invoker to one of these from an `init` slot, and background code reports back
// through it (the invoker queues the call onto Qt's event loop).
pub struct Remote(Mutex<Option<QmlMethodInvoker>>);

impl Remote {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }

    pub fn set(&self, invoker: QmlMethodInvoker) {
        *self.0.lock().expect("remote lock poisoned") = Some(invoker);
    }

    // No-op until the store has registered itself
    pub fn with(&self, call: impl FnOnce(&QmlMethodInvoker)) {
        if let Some(invoker) = self.0.lock().expect("remote lock poisoned").as_ref() {
            call(invoker);
        }
    }
}
