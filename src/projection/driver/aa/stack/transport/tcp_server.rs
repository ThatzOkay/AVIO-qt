//! TCP server that accepts Android Auto (wireless) connections on port 5277.
//!
//! This is the *wireless* transport. The wired (AOAP) path bridges USB bulk endpoints to a
//! loopback TCP port instead (see `usb_aoap_bridge.rs`) and dials it separately.

use tokio::net::TcpListener;
use tokio::sync::{mpsc, Notify};

use super::super::session::config::SessionConfig;
use super::super::session::session::{Session, SessionCommand, SessionEvent};

pub struct TcpServer {
    cfg: SessionConfig,
}

impl TcpServer {
    pub fn new(cfg: SessionConfig) -> Self {
        Self { cfg }
    }

    /// Listens for incoming AA connections, spawning a `Session` per connection. Each session
    /// forwards its events onto the shared `events` channel. Runs until the listener errors.
    pub async fn listen(
        &self,
        port: u16,
        events: mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        let listener = TcpListener::bind(("0.0.0.0", port)).await?;
        println!("[TcpServer] listening on port {port}");

        loop {
            let (socket, addr) = listener.accept().await?;
            println!("[TcpServer] connection from {addr}");
            let _ = socket.set_nodelay(true);

            let session = Session::new(socket, self.cfg.clone());
            let events = events.clone();
            // Keep the sender alive for the session's lifetime so `commands.recv()` blocks
            // instead of immediately returning `None` on every poll. No shutdown source wired
            // up for the wireless path yet, so this Notify is never triggered.
            let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<SessionCommand>();
            let shutdown = Notify::new();
            tokio::spawn(async move {
                let _cmd_tx = cmd_tx;
                session.run(events, cmd_rx, &shutdown).await;
            });
        }
    }
}
