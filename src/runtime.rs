use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::Handle;
use tokio::task::JoinHandle;

// Qt owns the main thread, so there's no ambient tokio runtime to call tokio::spawn on from there.
// main stores a handle to the runtime it builds; anything can spawn onto it from any thread.
static HANDLE: OnceLock<Handle> = OnceLock::new();

pub fn init(handle: Handle) {
    HANDLE.set(handle).expect("runtime::init called twice");
}

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    HANDLE.get().expect("runtime::init not called").spawn(future)
}
