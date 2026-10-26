//! USB <-> TCP-loopback bridge for wired Android Auto.
//!
//! The phone, after the AOAP handshake, exposes two bulk USB endpoints carrying the AA byte
//! stream. This bridge performs the handshake (if needed), waits for the phone to re-enumerate
//! in accessory mode, claims the bulk interface, and pumps bytes between it and a loopback TCP
//! socket that the rest of the AA stack (`stack::transport::TcpServer`) connects to.
//!
//! Lifecycle: `start()` → handshake (if needed) + claim accessory interface + listen on the
//! loopback port. A single client may be connected at a time; `stop()` tears the pump down and
//! resets the device so the phone re-enumerates back to its normal (non-accessory) mode.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_lite::StreamExt;
use nusb::descriptors::TransferType;
use nusb::io::{EndpointRead, EndpointWrite};
use nusb::transfer::{Bulk, Direction, In, Out};
use nusb::{hotplug::HotplugEvent, Device, DeviceInfo, Interface};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::task::JoinHandle;

use super::super::aoap::constants::{AOAP_LOOPBACK_HOST, AOAP_RE_ENUMERATE_TIMEOUT_MS};
use super::super::aoap::handshake::{is_accessory_mode, run_aoap_handshake, HandshakeError};

const BULK_TRANSFER_SIZE: usize = 16 * 1024;

#[derive(Debug)]
pub enum BridgeEvent {
    Ready { host: String, port: u16 },
    Error(String),
    Closed,
}

#[derive(Debug)]
pub enum BridgeError {
    Handshake(HandshakeError),
    Usb(nusb::Error),
    Io(io::Error),
    ReenumerateTimeout,
    WatcherClosed,
    NoBulkEndpoints,
    AlreadyRunning,
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Handshake(e) => write!(f, "AOAP handshake failed: {e}"),
            BridgeError::Usb(e) => write!(f, "USB error: {e}"),
            BridgeError::Io(e) => write!(f, "I/O error: {e}"),
            BridgeError::ReenumerateTimeout => {
                write!(f, "phone did not re-enumerate in accessory mode in time")
            }
            BridgeError::WatcherClosed => write!(f, "USB hotplug watcher closed unexpectedly"),
            BridgeError::NoBulkEndpoints => {
                write!(f, "AOAP accessory: bulk IN/OUT endpoints not found")
            }
            BridgeError::AlreadyRunning => write!(f, "bridge is already running"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl From<io::Error> for BridgeError {
    fn from(e: io::Error) -> Self {
        BridgeError::Io(e)
    }
}

/// Combines a USB bulk IN reader and bulk OUT writer into one duplex stream so it can be pumped
/// against the TCP loopback socket with `tokio::io::copy_bidirectional`.
struct UsbDuplex {
    reader: EndpointRead<Bulk>,
    writer: EndpointWrite<Bulk>,
}

impl AsyncRead for UsbDuplex {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl AsyncWrite for UsbDuplex {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}

pub struct UsbAoapBridge {
    running: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl UsbAoapBridge {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(Notify::new()),
            task: Mutex::new(None),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub async fn start(
        &self,
        phone: DeviceInfo,
        port: u16,
        events: mpsc::UnboundedSender<BridgeEvent>,
    ) -> Result<(), BridgeError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(BridgeError::AlreadyRunning);
        }

        let setup = async {
            let accessory_info = switch_and_wait_for_accessory(&phone).await?;
            let device = accessory_info.open().await.map_err(BridgeError::Usb)?;
            let (interface, in_addr, out_addr) = claim_bulk_interface(&device).await?;
            Ok::<_, BridgeError>((device, interface, in_addr, out_addr))
        }
        .await;

        let (device, interface, in_addr, out_addr) = match setup {
            Ok(v) => v,
            Err(e) => {
                self.running.store(false, Ordering::SeqCst);
                return Err(e);
            }
        };

        let listener = TcpListener::bind((AOAP_LOOPBACK_HOST, port)).await?;
        let host = AOAP_LOOPBACK_HOST.to_string();

        let running = self.running.clone();
        let shutdown = self.shutdown.clone();
        let task = tokio::spawn(async move {
            let _ = events.send(BridgeEvent::Ready { host, port });
            run_bridge(listener, device, interface, in_addr, out_addr, &shutdown).await;
            running.store(false, Ordering::SeqCst);
            let _ = events.send(BridgeEvent::Closed);
        });

        *self.task.lock().await = Some(task);
        Ok(())
    }

    pub async fn stop(&self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }
        // `notify_one`, not `notify_waiters`: the latter only wakes a task that is already
        // polling `.notified()`, so a `stop()` landing between loop iterations in `run_bridge`
        // would be silently missed. `notify_one` stores a permit when no one is waiting yet,
        // which is exactly what a single-consumer shutdown signal needs.
        self.shutdown.notify_one();
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }
}

impl Default for UsbAoapBridge {
    fn default() -> Self {
        Self::new()
    }
}

async fn switch_and_wait_for_accessory(phone: &DeviceInfo) -> Result<DeviceInfo, BridgeError> {
    if is_accessory_mode(phone) {
        return Ok(phone.clone());
    }

    let watcher = nusb::watch_devices().map_err(BridgeError::Usb)?;

    let device = phone.open().await.map_err(BridgeError::Usb)?;
    let handshake_result = run_aoap_handshake(&device).await;
    drop(device); // release before the phone disconnects and re-enumerates
    handshake_result.map_err(BridgeError::Handshake)?;

    wait_for_accessory_attach(watcher, Duration::from_millis(AOAP_RE_ENUMERATE_TIMEOUT_MS)).await
}

async fn wait_for_accessory_attach(
    mut watcher: nusb::hotplug::HotplugWatch,
    timeout: Duration,
) -> Result<DeviceInfo, BridgeError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(BridgeError::ReenumerateTimeout);
        }
        match tokio::time::timeout(remaining, watcher.next()).await {
            Ok(Some(HotplugEvent::Connected(info))) if is_accessory_mode(&info) => {
                return Ok(info);
            }
            Ok(Some(_)) => continue,
            Ok(None) => return Err(BridgeError::WatcherClosed),
            Err(_) => return Err(BridgeError::ReenumerateTimeout),
        }
    }
}

async fn claim_bulk_interface(device: &Device) -> Result<(Interface, u8, u8), BridgeError> {
    let active_config = device
        .active_configuration()
        .map(|c| c.configuration_value())
        .unwrap_or(0);
    if active_config != 1 {
        let _ = device.set_configuration(1).await;
    }

    let (iface_number, in_addr, out_addr) = find_bulk_endpoints(device)?;
    let interface = device
        .claim_interface(iface_number)
        .await
        .map_err(BridgeError::Usb)?;
    Ok((interface, in_addr, out_addr))
}

fn find_bulk_endpoints(device: &Device) -> Result<(u8, u8, u8), BridgeError> {
    let config = device
        .active_configuration()
        .map_err(|_| BridgeError::NoBulkEndpoints)?;

    for group in config.interfaces() {
        let alt = group.first_alt_setting();
        let mut in_addr = None;
        let mut out_addr = None;
        for ep in alt.endpoints() {
            if ep.transfer_type() != TransferType::Bulk {
                continue;
            }
            match ep.direction() {
                Direction::In => in_addr = Some(ep.address()),
                Direction::Out => out_addr = Some(ep.address()),
            }
        }
        if let (Some(in_addr), Some(out_addr)) = (in_addr, out_addr) {
            return Ok((group.interface_number(), in_addr, out_addr));
        }
    }

    Err(BridgeError::NoBulkEndpoints)
}

async fn run_bridge(
    listener: TcpListener,
    device: Device,
    interface: Interface,
    in_addr: u8,
    out_addr: u8,
    shutdown: &Notify,
) {
    'accept: loop {
        let (mut socket, _) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(v) => v,
                Err(_) => break 'accept,
            },
            _ = shutdown.notified() => break 'accept,
        };
        let _ = socket.set_nodelay(true);

        let (reader, writer) = match (
            interface.endpoint::<Bulk, In>(in_addr),
            interface.endpoint::<Bulk, Out>(out_addr),
        ) {
            (Ok(in_ep), Ok(out_ep)) => (
                in_ep.reader(BULK_TRANSFER_SIZE),
                out_ep.writer(BULK_TRANSFER_SIZE),
            ),
            _ => break 'accept,
        };
        let mut usb_stream = UsbDuplex { reader, writer };

        tokio::select! {
            _ = tokio::io::copy_bidirectional(&mut socket, &mut usb_stream) => {}
            _ = shutdown.notified() => break 'accept,
        }
    }

    // Release the claimed interface before resetting so the reset isn't fighting an open claim,
    // then nudge the phone back to its normal (non-accessory) USB mode.
    drop(interface);
    let _ = device.reset().await;
}
