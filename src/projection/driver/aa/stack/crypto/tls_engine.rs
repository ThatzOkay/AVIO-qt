//! In-memory TLS engine for the Android Auto protocol.
//!
//! AA tunnels TLS bytes through its own framing instead of a real socket:
//!   - During handshake: TLS bytes travel in SSL_HANDSHAKE frames (msgId 0x0003)
//!   - After handshake:  each frame payload IS one or more TLS records
//!
//! `MemStream` is a pair of in-memory byte queues standing in for the socket OpenSSL expects;
//! feed ciphertext in, read plaintext out. Roles: Phone = TLS server, HU (us) = TLS client.

use std::collections::VecDeque;
use std::io::{self, Read, Write};

use openssl::pkey::PKey;
use openssl::ssl::{
    HandshakeError, MidHandshakeSslStream, Ssl, SslContext, SslMethod, SslStream, SslVerifyMode,
    SslVersion,
};
use openssl::x509::X509;

use super::cert::{HU_CERT_PEM, HU_KEY_PEM};

/// In-memory duplex "transport" for OpenSSL to read/write ciphertext against.
#[derive(Default)]
struct MemStream {
    incoming: VecDeque<u8>,
    outgoing: Vec<u8>,
}

impl Read for MemStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.incoming.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "no ciphertext buffered",
            ));
        }
        let n = buf.len().min(self.incoming.len());
        for slot in buf.iter_mut().take(n) {
            *slot = self.incoming.pop_front().expect("checked len above");
        }
        Ok(n)
    }
}

impl Write for MemStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.outgoing.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn build_ssl() -> Result<Ssl, openssl::error::ErrorStack> {
    let mut ctx = SslContext::builder(SslMethod::tls_client())?;
    ctx.set_min_proto_version(Some(SslVersion::TLS1_2))?;
    ctx.set_max_proto_version(Some(SslVersion::TLS1_2))?;
    // The phone presents a self-signed cert with no real chain, and we don't check its identity
    // either (there's no DNS name in play) — matches the reference's
    // `rejectUnauthorized: false, checkServerIdentity: () => undefined`.
    ctx.set_verify(SslVerifyMode::NONE);

    let cert = X509::from_pem(HU_CERT_PEM.as_bytes())?;
    let key = PKey::private_key_from_pem(HU_KEY_PEM.as_bytes())?;
    ctx.set_certificate(&cert)?;
    ctx.set_private_key(&key)?;

    Ssl::new(&ctx.build())
}

/// Bytes produced by feeding data into the TLS engine.
#[derive(Default)]
pub struct TlsFeedResult {
    /// Decrypted application data, if any completed records were ready.
    pub plaintext: Vec<u8>,
    /// New ciphertext (or handshake flight) the engine wants sent to the phone.
    pub outbound: Vec<u8>,
}

enum State {
    Handshaking(MidHandshakeSslStream<MemStream>),
    Connected(SslStream<MemStream>),
    /// The handshake failed; every call is now a no-op until the session is torn down.
    Failed,
}

pub struct TlsEngine {
    state: State,
}

impl TlsEngine {
    pub fn new() -> Result<Self, openssl::error::ErrorStack> {
        let ssl = build_ssl()?;
        // The initial connect() writes the ClientHello into the (still empty) stream and then
        // finds no ServerHello to read yet, so it always comes back as WouldBlock here — this is
        // also how we pick up the initial outbound flight (see take_initial_outbound).
        let state = match ssl.connect(MemStream::default()) {
            Ok(stream) => State::Connected(stream),
            Err(HandshakeError::WouldBlock(mid)) => State::Handshaking(mid),
            Err(_) => State::Failed,
        };
        Ok(Self { state })
    }

    pub fn is_handshaking(&self) -> bool {
        matches!(self.state, State::Handshaking(_))
    }

    /// Feed raw bytes received from the phone (handshake bytes or one-or-more TLS records) into
    /// the engine, returning any decrypted application data and any outbound bytes the engine
    /// wants sent back (handshake flight, alerts, ...).
    pub fn feed(&mut self, incoming: &[u8]) -> io::Result<TlsFeedResult> {
        match std::mem::replace(&mut self.state, State::Failed) {
            State::Handshaking(mut mid) => {
                mid.get_mut().incoming.extend(incoming.iter().copied());
                match mid.handshake() {
                    Ok(mut stream) => {
                        let outbound = std::mem::take(&mut stream.get_mut().outgoing);
                        self.state = State::Connected(stream);
                        Ok(TlsFeedResult {
                            plaintext: Vec::new(),
                            outbound,
                        })
                    }
                    Err(HandshakeError::WouldBlock(mut mid)) => {
                        let outbound = std::mem::take(&mut mid.get_mut().outgoing);
                        self.state = State::Handshaking(mid);
                        Ok(TlsFeedResult {
                            plaintext: Vec::new(),
                            outbound,
                        })
                    }
                    Err(HandshakeError::Failure(mid)) => {
                        Err(io::Error::other(mid.error().to_string()))
                    }
                    Err(HandshakeError::SetupFailure(e)) => Err(io::Error::other(e.to_string())),
                }
            }
            State::Connected(mut stream) => {
                stream.get_mut().incoming.extend(incoming.iter().copied());
                let plaintext = drain_plaintext(&mut stream);
                let outbound = std::mem::take(&mut stream.get_mut().outgoing);
                self.state = State::Connected(stream);
                Ok(TlsFeedResult {
                    plaintext,
                    outbound,
                })
            }
            State::Failed => Err(io::Error::other("TLS handshake previously failed")),
        }
    }

    /// Encrypt cleartext application data, returning the resulting ciphertext to send.
    pub fn encrypt(&mut self, cleartext: &[u8]) -> io::Result<Vec<u8>> {
        match &mut self.state {
            State::Connected(stream) => {
                stream.write_all(cleartext)?;
                Ok(std::mem::take(&mut stream.get_mut().outgoing))
            }
            _ => Err(io::Error::other("TLS handshake not complete")),
        }
    }

    /// Drain any bytes the engine wants to send without feeding it anything first — used right
    /// after construction to pick up the initial ClientHello flight.
    pub fn take_initial_outbound(&mut self) -> io::Result<Vec<u8>> {
        match &mut self.state {
            State::Handshaking(mid) => Ok(std::mem::take(&mut mid.get_mut().outgoing)),
            State::Connected(stream) => Ok(std::mem::take(&mut stream.get_mut().outgoing)),
            State::Failed => Err(io::Error::other("TLS handshake previously failed")),
        }
    }
}

fn drain_plaintext(stream: &mut SslStream<MemStream>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => break, // unclean EOF — nothing more usable this call
        }
    }
    out
}
