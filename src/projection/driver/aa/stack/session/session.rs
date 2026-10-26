//! AA wireless session — one per TCP connection.
//!
//! State: Init -> Version -> TlsHandshake -> Auth -> ServiceDiscovery -> ChannelSetup -> Running
//!        -> Closed
//!
//! Channels other than the control channel and the generic AV-setup handshake are not built yet
//! (Video/Audio/Mic/Input/Navigation/MediaInfo — see the channel milestones); messages destined
//! for them are logged and dropped for now rather than acted on.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time;

use aa_proto::aap_protobuf::service::bluetooth::message::{
    BluetoothPairingRequest, BluetoothPairingResponse,
};
use aa_proto::aap_protobuf::service::control::message::{
    AuthResponse, ChannelOpenResponse, PingRequest,
};
use aa_proto::aap_protobuf::shared::MessageStatus;
use aa_proto::oaa::proto::messages::{AvChannelSetupRequest, AvChannelSetupResponse};

use crate::projection::driver::aa::stack::channels::radio_channel::RadioChannel;

use super::super::channels::audio_channel::{AudioChannel, AudioChannelType, AudioEvent};
use super::super::channels::input_channel::{self, TouchPointer};
use super::super::channels::media_info_channel::{self, MediaInfoEvent};
use super::super::channels::mic_channel::{MicChannel, MicEvent};
use super::super::channels::navigation_channel::{self, NavEvent};
use super::super::channels::video_channel::{VideoChannel, VideoEvent};
use super::super::constants::{
    av_msg, av_setup_status, ch, ctrl_msg, frame_flags, media_codec, version,
    STATUS_OK,
};
use super::super::crypto::tls_engine::TlsEngine;
use super::super::frame::codec::{encode_frame, FrameParser, RawFrame};
use super::config::{SessionConfig, VideoCodec};
use super::control_channel::{ControlChannel, ControlEvent};
use super::service_discovery::build_service_discovery_response;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SessionState {
    Init,
    Version,
    TlsHandshake,
    Auth,
    ServiceDiscovery,
    ChannelSetup,
    Running,
    Closed,
}

#[derive(Debug)]
pub enum SessionEvent {
    Connected,
    Disconnected,
    Error(String),
    /// Phone's self-reported name, arrives with the `ServiceDiscoveryRequest` — before
    /// `Connected` (which only fires once AV channel setup completes).
    DeviceInfo { device_name: Option<String> },
    /// Encoded video access unit ready for the decoder. `channel_id` is `ch::VIDEO` or
    /// `ch::CLUSTER_VIDEO`; `codec` is whichever of H264/H265/VP9/AV1 was negotiated for it.
    VideoFrame {
        channel_id: u8,
        codec: VideoCodec,
        data: Vec<u8>,
        timestamp_ns: u64,
    },
    /// Negotiated once per SDR: the video tier's full encoded size plus the actual AA content
    /// region within it (crop out the phone's own letterbox bars, scale the rest to fill the
    /// screen — see `GstVideo::set_content_region`).
    VideoGeometry {
        crop_left: u32,
        crop_top: u32,
        vis_width: u32,
        vis_height: u32,
        tier_width: u32,
        tier_height: u32,
        // The touchscreen space advertised in the same SDR — see `send_touch`'s doc comment on
        // why this can differ from `vis_width`/`vis_height`.
        touch_width: u32,
        touch_height: u32,
    },
    HostUiRequested,
    /// PCM/AAC-LC audio from the phone. `channel_id` is one of `ch::MEDIA_AUDIO`,
    /// `ch::SPEECH_AUDIO`, `ch::SYSTEM_AUDIO`.
    AudioFrame {
        channel_id: u8,
        data: Vec<u8>,
        timestamp_ns: u64,
    },
    AudioStart {
        channel_id: u8,
    },
    AudioStop {
        channel_id: u8,
    },
    /// Phone asked the HU to begin/stop sending mic PCM (see `Session::push_mic_pcm`).
    MicStart,
    MicStop,
    NavStart,
    NavStop,
    NavStatus(navigation_channel::NavigationStatusUpdate),
    NavTurn(navigation_channel::NavigationTurnUpdate),
    NavDistance(navigation_channel::NavigationDistanceUpdate),
    NavState(navigation_channel::NavigationStateUpdate),
    NavPosition(navigation_channel::NavigationPositionUpdate),
    MediaMetadata(media_info_channel::MediaPlaybackMetadata),
    MediaStatus(media_info_channel::MediaPlaybackStatus),
}

/// Commands sent into a running `Session` from outside (e.g. touch input captured by the UI).
#[derive(Debug)]
pub enum SessionCommand {
    /// Single-pointer touch in advertised touchscreen-space pixels (see `Session::send_touch`).
    Touch { action: u32, x: u32, y: u32 },
    /// Multi-pointer touch in advertised touchscreen-space pixels (see `Session::send_touch`).
    /// `points` carries each pointer's stable per-finger id (matching the browser's
    /// `Touch.identifier`) alongside its coordinates, and `action_index` is the position within
    /// `points` of whichever pointer triggered `action` — required for POINTER_DOWN/POINTER_UP.
    MultiTouch {
        action: u32,
        points: Vec<(u32, u32, u32)>,
        action_index: u32,
    },
    /// HW button/key event (see `Session::send_button`).
    Button {
        key_codes: Vec<u32>,
        down: bool,
        longpress: bool,
    },
    /// Rotary-encoder delta event (see `Session::send_rotary`).
    Rotary { direction: i32 },
    /// Resume projected AA content after the phone requested the host UI (see
    /// `Session::request_video_focus`).
    RequestVideoFocus,
}

struct CleartextFragment {
    parts: Vec<u8>,
    flags: u8,
}

pub struct Session {
    socket: TcpStream,
    cfg: SessionConfig,
    state: SessionState,
    raw_parser: FrameParser,
    tls: Option<TlsEngine>,
    tls_buf: Vec<u8>,
    control: ControlChannel,
    video: VideoChannel,
    cluster: VideoChannel,
    audio: HashMap<u8, AudioChannel>,
    mic: MicChannel,
    radio: RadioChannel,
    video_codec_by_index: Vec<VideoCodec>,
    cluster_codec_by_index: Vec<VideoCodec>,
    video_codec: Option<VideoCodec>,
    cluster_codec: Option<VideoCodec>,
    cleartext_fragments: HashMap<u8, CleartextFragment>,
    last_pong_at: Instant,
    // Rolling trace of the last few encrypted frames fed to TLS (channel_id, flags, len),
    // dumped if a feed() ever fails — helps spot a record-ordering/sequencing bug that a single
    // failing frame in isolation can't show.
    #[allow(dead_code)]
    recent_frames: std::collections::VecDeque<(u8, u8, usize)>,
}

impl Session {
    pub fn new(socket: TcpStream, cfg: SessionConfig) -> Self {
        let _ = socket.set_nodelay(true);
        Self {
            socket,
            cfg,
            state: SessionState::Init,
            raw_parser: FrameParser::new(),
            tls: None,
            tls_buf: Vec::new(),
            control: ControlChannel,
            video: VideoChannel::new(ch::VIDEO),
            cluster: VideoChannel::new(ch::CLUSTER_VIDEO),
            audio: HashMap::from([
                (
                    ch::MEDIA_AUDIO,
                    AudioChannel::new(ch::MEDIA_AUDIO, AudioChannelType::Media),
                ),
                (
                    ch::SPEECH_AUDIO,
                    AudioChannel::new(ch::SPEECH_AUDIO, AudioChannelType::Speech),
                ),
                (
                    ch::SYSTEM_AUDIO,
                    AudioChannel::new(ch::SYSTEM_AUDIO, AudioChannelType::System),
                ),
            ]),
            mic: MicChannel::new(ch::MIC_INPUT),
            radio: RadioChannel::new(ch::RADIO),
            video_codec_by_index: Vec::new(),
            cluster_codec_by_index: Vec::new(),
            video_codec: None,
            cluster_codec: None,
            cleartext_fragments: HashMap::new(),
            last_pong_at: Instant::now(),
            recent_frames: std::collections::VecDeque::new(),
        }
    }

    /// Drives the session to completion, sending `SessionEvent`s as they happen and applying
    /// `SessionCommand`s (touch/button/rotary input) as they arrive on `commands`. Returns once
    /// the phone disconnects or the session is closed for any reason.
    pub async fn run(
        mut self,
        events: mpsc::UnboundedSender<SessionEvent>,
        mut commands: mpsc::UnboundedReceiver<SessionCommand>,
        shutdown: &tokio::sync::Notify,
    ) {
        if let Err(e) = self.send_version_request().await {
            let _ = events.send(SessionEvent::Error(e.to_string()));
            return;
        }
        self.state = SessionState::Version;

        let mut ping_interval = time::interval(Duration::from_millis(1500));
        ping_interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        let watchdog = time::sleep(Duration::from_secs(30));
        tokio::pin!(watchdog);
        let mut buf = vec![0u8; 16 * 1024];

        loop {
            tokio::select! {
                result = self.socket.read(&mut buf) => {
                    match result {
                        Ok(0) => break,
                        Ok(n) => {
                            let chunk = buf[..n].to_vec();
                            if let Err(e) = self.handle_incoming(&chunk, &events).await {
                                let _ = events.send(SessionEvent::Error(e.to_string()));
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = events.send(SessionEvent::Error(e.to_string()));
                            break;
                        }
                    }
                }
                _ = ping_interval.tick(), if self.state >= SessionState::ServiceDiscovery => {
                    if self.last_pong_at.elapsed() > Duration::from_millis(5000) {
                        let _ = events.send(SessionEvent::Error("ping timeout".to_string()));
                        break;
                    }
                    if let Err(e) = self.send_ping().await {
                        let _ = events.send(SessionEvent::Error(e.to_string()));
                        break;
                    }
                }
                () = &mut watchdog, if self.state < SessionState::Running => {
                    let _ = events.send(SessionEvent::Error(
                        "session stalled in pre-RUNNING state — phone-side AA service likely zombie".to_string(),
                    ));
                    break;
                }
                () = shutdown.notified() => {
                    println!("[Session] shutdown requested, closing");
                    break;
                }
                command = commands.recv() => {
                    let Some(command) = command else {
                        // Sender dropped — no more input will ever arrive, but the phone
                        // connection itself is unaffected, so keep the session running.
                        continue;
                    };
                    let result = match command {
                        SessionCommand::Touch { action, x, y } => {
                            self.send_touch(action, &[TouchPointer { x, y, id: 0 }], 0).await
                        }
                        SessionCommand::MultiTouch { action, points, action_index } => {
                            let pointers: Vec<TouchPointer> = points
                                .into_iter()
                                .map(|(x, y, id)| TouchPointer { x, y, id })
                                .collect();
                            self.send_touch(action, &pointers, action_index).await
                        }
                        SessionCommand::Button { key_codes, down, longpress } => {
                            self.send_button(&key_codes, down, longpress).await
                        }
                        SessionCommand::Rotary { direction } => self.send_rotary(direction).await,
                        SessionCommand::RequestVideoFocus => self.request_video_focus().await,
                    };
                    if let Err(e) = result {
                        let _ = events.send(SessionEvent::Error(e.to_string()));
                        break;
                    }
                }
            }

            if self.state == SessionState::Closed {
                break;
            }
        }

        let _ = events.send(SessionEvent::Disconnected);
    }

    // ── Incoming byte routing ─────────────────────────────────────────────────

    async fn handle_incoming(
        &mut self,
        chunk: &[u8],
        events: &mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        if self.state <= SessionState::TlsHandshake {
            let frames = self.raw_parser.push(chunk);
            for frame in frames {
                self.handle_raw_frame(frame, events).await?;
            }
        } else {
            self.strip_header_and_inject_tls(chunk, events).await?;
        }
        Ok(())
    }

    // ── Pre-auth frame handling (VERSION_RESPONSE / SSL_HANDSHAKE) ───────────

    async fn handle_raw_frame(
        &mut self,
        frame: RawFrame,
        events: &mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        match frame.msg_id {
            ctrl_msg::VERSION_RESPONSE => self.on_version_response(&frame.payload).await,
            ctrl_msg::SSL_HANDSHAKE => self.on_ssl_handshake(&frame.payload).await,
            _ => {
                // An encrypted frame piggy-backed on the same TCP segment as TLS Finished.
                if self.tls.is_some() && frame.flags & 0x08 != 0 {
                    self.on_encrypted_frame(
                        frame.channel_id,
                        frame.flags,
                        &frame.raw_payload,
                        events,
                    )
                    .await
                } else {
                    Ok(())
                }
            }
        }
    }

    async fn on_version_response(&mut self, payload: &[u8]) -> std::io::Result<()> {
        if payload.len() < 6 {
            return Ok(());
        }
        let status = u16::from_be_bytes([payload[4], payload[5]]);
        if status == version::STATUS_MISMATCH {
            self.state = SessionState::Closed;
            return Ok(());
        }

        self.state = SessionState::TlsHandshake;
        let mut tls = TlsEngine::new().map_err(io_err)?;
        let initial = tls.take_initial_outbound().map_err(io_err)?;
        self.tls = Some(tls);
        if !initial.is_empty() {
            self.send_handshake_bytes(&initial).await?;
        }
        Ok(())
    }

    async fn on_ssl_handshake(&mut self, payload: &[u8]) -> std::io::Result<()> {
        let Some(tls) = self.tls.as_mut() else {
            return Ok(());
        };
        let was_handshaking = tls.is_handshaking();
        let result = tls.feed(payload)?;
        if !result.outbound.is_empty() {
            self.send_handshake_bytes(&result.outbound).await?;
        }
        if was_handshaking && !self.tls.as_ref().unwrap().is_handshaking() {
            self.on_secure_connect().await?;
        }
        Ok(())
    }

    async fn on_secure_connect(&mut self) -> std::io::Result<()> {
        self.state = SessionState::Auth;
        let auth_buf = AuthResponse { status: STATUS_OK }.encode_to_vec();
        self.send_aa(
            ch::CONTROL,
            frame_flags::PLAINTEXT,
            ctrl_msg::AUTH_COMPLETE,
            &auth_buf,
        )
        .await?;
        self.state = SessionState::ServiceDiscovery;
        Ok(())
    }

    async fn send_handshake_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let frame = encode_frame(
            ch::CONTROL,
            frame_flags::PLAINTEXT,
            ctrl_msg::SSL_HANDSHAKE,
            bytes,
        );
        self.socket.write_all(&frame).await
    }

    // ── Post-auth frame handling ──────────────────────────────────────────────

    async fn strip_header_and_inject_tls(
        &mut self,
        chunk: &[u8],
        events: &mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        self.tls_buf.extend_from_slice(chunk);

        loop {
            if self.tls_buf.len() < 4 {
                break;
            }
            let channel_id = self.tls_buf[0];
            let flags = self.tls_buf[1];
            let is_encrypted = flags & 0x08 != 0;
            let is_first = flags & 0x01 != 0;
            let is_last = flags & 0x02 != 0;
            let is_extended = is_first && !is_last;
            let header_len = if is_extended { 8 } else { 4 };

            if self.tls_buf.len() < header_len {
                break;
            }
            let payload_size = u16::from_be_bytes([self.tls_buf[2], self.tls_buf[3]]) as usize;
            let total_len = header_len + payload_size;
            if self.tls_buf.len() < total_len {
                break;
            }

            let raw_payload = self.tls_buf[header_len..total_len].to_vec();
            self.tls_buf.drain(0..total_len);

            if !is_encrypted {
                if raw_payload.len() < 2 {
                    continue;
                }
                let msg_id = u16::from_be_bytes([raw_payload[0], raw_payload[1]]);
                let payload = raw_payload[2..].to_vec();
                self.handle_decrypted_message(channel_id, flags, msg_id, &payload, events)
                    .await?;
                continue;
            }

            self.on_encrypted_frame(channel_id, flags, &raw_payload, events)
                .await?;
        }
        Ok(())
    }

    async fn on_encrypted_frame(
        &mut self,
        channel_id: u8,
        flags: u8,
        raw_payload: &[u8],
        events: &mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        let Some(tls) = self.tls.as_mut() else {
            return Ok(());
        };
        let result = match tls.feed(raw_payload) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "[Session] tls.feed FAILED ch={channel_id} flags=0x{flags:02x} len={} first_bytes={:02x?}: {e}",
                    raw_payload.len(),
                    &raw_payload[..raw_payload.len().min(16)]
                );
                return Err(e);
            }
        };
        if result.plaintext.is_empty() {
            return Ok(());
        }

        let is_first = flags & 0x01 != 0;
        let is_last = flags & 0x02 != 0;

        if is_first && is_last {
            if result.plaintext.len() < 2 {
                return Ok(());
            }
            let msg_id = u16::from_be_bytes([result.plaintext[0], result.plaintext[1]]);
            let payload = result.plaintext[2..].to_vec();
            return self
                .handle_decrypted_message(channel_id, flags, msg_id, &payload, events)
                .await;
        }

        if is_first && !is_last {
            println!(
                "[Session] fragment START ch={channel_id} first_chunk_len={}",
                result.plaintext.len()
            );
            self.cleartext_fragments.insert(
                channel_id,
                CleartextFragment {
                    parts: result.plaintext,
                    flags,
                },
            );
            return Ok(());
        }

        let Some(state) = self.cleartext_fragments.get_mut(&channel_id) else {
            println!("[Session] fragment CONTINUE/END ch={channel_id} but no in-progress fragment — dropping {}B", result.plaintext.len());
            return Ok(());
        };
        state.parts.extend_from_slice(&result.plaintext);

        if is_last {
            let state = self
                .cleartext_fragments
                .remove(&channel_id)
                .expect("checked above");
            println!(
                "[Session] fragment END ch={channel_id} total_len={}",
                state.parts.len()
            );
            if state.parts.len() < 2 {
                return Ok(());
            }
            let msg_id = u16::from_be_bytes([state.parts[0], state.parts[1]]);
            let payload = state.parts[2..].to_vec();
            return self
                .handle_decrypted_message(channel_id, state.flags, msg_id, &payload, events)
                .await;
        }
        Ok(())
    }

    async fn handle_decrypted_message(
        &mut self,
        channel_id: u8,
        _flags: u8,
        msg_id: u16,
        payload: &[u8],
        events: &mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        // Any incoming frame counts as liveness.
        self.last_pong_at = Instant::now();

        if channel_id == ch::CONTROL {
            let mut outbox: Vec<(u8, u8, u16, Vec<u8>)> = Vec::new();
            let mut send = |c: u8, f: u8, m: u16, d: &[u8]| outbox.push((c, f, m, d.to_vec()));
            let event = self.control.handle_message(msg_id, payload, &mut send);
            for (c, f, m, d) in outbox {
                self.send_aa(c, f, m, &d).await?;
            }
            return self.handle_control_event(event, events).await;
        }

        // Phone opens each service channel individually; we ack on the same channel. The HU
        // never initiates a channel open.
        if msg_id == ctrl_msg::CHANNEL_OPEN_REQUEST {
            let buf = ChannelOpenResponse { status: STATUS_OK }.encode_to_vec();
            return self
                .send_aa(
                    channel_id,
                    frame_flags::ENC_CONTROL,
                    ctrl_msg::CHANNEL_OPEN_RESPONSE,
                    &buf,
                )
                .await;
        }

        if msg_id == av_msg::SETUP_REQUEST {
            return self
                .handle_av_setup_request(channel_id, payload, events)
                .await;
        }

        if channel_id == ch::VIDEO || channel_id == ch::CLUSTER_VIDEO {
            return self
                .handle_video_message(channel_id, msg_id, payload, events)
                .await;
        }

        // Phone sends a list of keycodes it wants the HU to bind for input dispatch.
        if channel_id == ch::INPUT && msg_id == input_channel::input_msg::KEY_BINDING_REQUEST {
            // KeyBindingResponse: required int32 status = 1; varint tag 0x08, value 0 (OK).
            return self
                .send_aa(
                    ch::INPUT,
                    frame_flags::ENC_SIGNAL,
                    input_channel::input_msg::KEY_BINDING_RESPONSE,
                    &[0x08, 0x00],
                )
                .await;
        }

        if self.audio.contains_key(&channel_id) {
            return self
                .handle_audio_message(channel_id, msg_id, payload, events)
                .await;
        }

        if channel_id == ch::MIC_INPUT {
            return self.handle_mic_message(msg_id, payload, events).await;
        }

        if channel_id == ch::NAVIGATION {
            let event = match navigation_channel::handle_message(msg_id, payload) {
                NavEvent::Start => Some(SessionEvent::NavStart),
                NavEvent::Stop => Some(SessionEvent::NavStop),
                NavEvent::Status(s) => Some(SessionEvent::NavStatus(s)),
                NavEvent::Turn(t) => Some(SessionEvent::NavTurn(t)),
                NavEvent::Distance(d) => Some(SessionEvent::NavDistance(d)),
                NavEvent::State(s) => Some(SessionEvent::NavState(s)),
                NavEvent::Position(p) => Some(SessionEvent::NavPosition(p)),
                NavEvent::None => None,
            };
            if let Some(event) = event {
                let _ = events.send(event);
            }
            return Ok(());
        }

        if channel_id == ch::MEDIA_INFO {
            let event = match media_info_channel::handle_message(msg_id, payload) {
                MediaInfoEvent::Metadata(m) => Some(SessionEvent::MediaMetadata(m)),
                MediaInfoEvent::Status(s) => Some(SessionEvent::MediaStatus(s)),
                MediaInfoEvent::None => None,
            };
            if let Some(event) = event {
                let _ = events.send(event);
            }
            return Ok(());
        }

        if channel_id == ch::SENSOR {
            return self.handle_sensor_message(msg_id, payload).await;
        }

        if channel_id == ch::BLUETOOTH {
            return self.handle_bluetooth_message(msg_id, payload).await;
        }

        if channel_id == ch::RADIO {
            self.radio.handle_message(msg_id, payload);
            return Ok(());
        }

        // PhoneStatus/WiFi channel bodies aren't implemented yet.
        println!(
            "[Session] unhandled ch={channel_id} msgId=0x{msg_id:04x} len={} (channel not yet implemented)",
            payload.len()
        );
        Ok(())
    }

    /// SENSOR_MESSAGE_REQUEST (0x8001): the phone asks to subscribe to a sensor type. We ack
    /// every type, but only a couple carry a meaningful initial value worth sending.
    async fn handle_sensor_message(&mut self, msg_id: u16, payload: &[u8]) -> std::io::Result<()> {
        if msg_id != 0x8001 {
            return Ok(());
        }

        // SensorRequest: field 1 (varint) = SensorType.
        let sensor_type = if payload.len() >= 2 && payload[0] == 0x08 {
            payload[1]
        } else {
            0
        };

        // SensorStartResponse: status=SUCCESS(0). msgId 0x8002 = SENSOR_MESSAGE_RESPONSE.
        self.send_aa(ch::SENSOR, frame_flags::ENC_SIGNAL, 0x8002, &[0x08, 0x00])
            .await?;

        // SensorBatch (msgId 0x8003) — emit an initial value for the types that need one to
        // avoid the phone treating the sensor as stuck at an unknown state.
        match sensor_type {
            13 => {
                // DrivingStatus = UNRESTRICTED(0)
                self.send_aa(
                    ch::SENSOR,
                    frame_flags::ENC_SIGNAL,
                    0x8003,
                    &[0x6a, 0x02, 0x08, 0x00],
                )
                .await?;
            }
            10 => {
                // NightMode
                let initial = self.cfg.initial_night_mode;
                self.send_aa(
                    ch::SENSOR,
                    frame_flags::ENC_SIGNAL,
                    0x8003,
                    &[0x52, 0x02, 0x08, initial as u8],
                )
                .await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// BLUETOOTH_MESSAGE_PAIRING_REQUEST (0x8001): the phone asks the HU to pair over
    /// Bluetooth. There's no Bluetooth stack wired in yet (that's the separate aa-bluetooth.py
    /// supervisor, not started), so decline honestly rather than claim a pairing that didn't
    /// happen.
    async fn handle_bluetooth_message(
        &mut self,
        msg_id: u16,
        payload: &[u8],
    ) -> std::io::Result<()> {
        if msg_id != 0x8001 {
            return Ok(());
        }
        if let Ok(req) = BluetoothPairingRequest::decode(payload) {
            println!(
                "[Session] BluetoothPairingRequest phone={} method={} — declining (no Bluetooth stack wired up yet)",
                req.phone_address, req.pairing_method
            );
        }
        let resp = BluetoothPairingResponse {
            status: MessageStatus::StatusBluetoothUnavailable as i32,
            already_paired: false,
        };
        self.send_aa(
            ch::BLUETOOTH,
            frame_flags::ENC_SIGNAL,
            0x8002,
            &resp.encode_to_vec(),
        )
        .await
    }

    async fn handle_video_message(
        &mut self,
        channel_id: u8,
        msg_id: u16,
        payload: &[u8],
        events: &mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        let mut outbox: Vec<(u8, u8, u16, Vec<u8>)> = Vec::new();
        let mut send = |c: u8, f: u8, m: u16, d: &[u8]| outbox.push((c, f, m, d.to_vec()));
        let event = if channel_id == ch::VIDEO {
            self.video.handle_message(msg_id, payload, &mut send)
        } else {
            self.cluster.handle_message(msg_id, payload, &mut send)
        };
        for (c, f, m, d) in outbox {
            self.send_aa(c, f, m, &d).await?;
        }

        match event {
            VideoEvent::Frame { data, timestamp_ns } => {
                let codec = if channel_id == ch::VIDEO {
                    self.video_codec
                } else {
                    self.cluster_codec
                }
                .unwrap_or(VideoCodec::H264);
                let _ = events.send(SessionEvent::VideoFrame {
                    channel_id,
                    codec,
                    data,
                    timestamp_ns,
                });
            }
            VideoEvent::HostUiRequested => {
                let _ = events.send(SessionEvent::HostUiRequested);
            }
            VideoEvent::VideoFocusProjected | VideoEvent::None => {}
        }
        Ok(())
    }

    async fn handle_audio_message(
        &mut self,
        channel_id: u8,
        msg_id: u16,
        payload: &[u8],
        events: &mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        let mut outbox: Vec<(u8, u8, u16, Vec<u8>)> = Vec::new();
        let mut send = |c: u8, f: u8, m: u16, d: &[u8]| outbox.push((c, f, m, d.to_vec()));
        let event = self
            .audio
            .get_mut(&channel_id)
            .expect("checked contains_key above")
            .handle_message(msg_id, payload, &mut send);
        for (c, f, m, d) in outbox {
            self.send_aa(c, f, m, &d).await?;
        }

        match event {
            AudioEvent::Pcm { data, timestamp_ns } => {
                let _ = events.send(SessionEvent::AudioFrame {
                    channel_id,
                    data,
                    timestamp_ns,
                });
            }
            AudioEvent::Start => {
                let _ = events.send(SessionEvent::AudioStart { channel_id });
            }
            AudioEvent::Stop => {
                let _ = events.send(SessionEvent::AudioStop { channel_id });
            }
            AudioEvent::None => {}
        }
        Ok(())
    }

    async fn handle_mic_message(
        &mut self,
        msg_id: u16,
        payload: &[u8],
        events: &mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        let mut outbox: Vec<(u8, u8, u16, Vec<u8>)> = Vec::new();
        let mut send = |c: u8, f: u8, m: u16, d: &[u8]| outbox.push((c, f, m, d.to_vec()));
        let event = self.mic.handle_message(msg_id, payload, &mut send);
        for (c, f, m, d) in outbox {
            self.send_aa(c, f, m, &d).await?;
        }

        match event {
            MicEvent::Start => {
                let _ = events.send(SessionEvent::MicStart);
            }
            MicEvent::Stop => {
                let _ = events.send(SessionEvent::MicStop);
            }
            MicEvent::None => {}
        }
        Ok(())
    }

    async fn handle_control_event(
        &mut self,
        event: ControlEvent,
        events: &mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        match event {
            ControlEvent::ServiceDiscoveryRequest(req) => {
                let _ = events.send(SessionEvent::DeviceInfo {
                    device_name: req.device_name,
                });
                let sdr = build_service_discovery_response(&self.cfg);
                self.video_codec_by_index = sdr.video_codec_by_index;
                self.cluster_codec_by_index = sdr.cluster_codec_by_index;
                let _ = events.send(SessionEvent::VideoGeometry {
                    crop_left: sdr.video_crop_left,
                    crop_top: sdr.video_crop_top,
                    vis_width: sdr.video_vis_width,
                    vis_height: sdr.video_vis_height,
                    tier_width: sdr.video_tier_width,
                    tier_height: sdr.video_tier_height,
                    touch_width: sdr.touch_width,
                    touch_height: sdr.touch_height,
                });
                self.send_aa(
                    ch::CONTROL,
                    frame_flags::ENC_SIGNAL,
                    ctrl_msg::SERVICE_DISCOVERY_RESPONSE,
                    &sdr.buf,
                )
                .await?;

                self.last_pong_at = Instant::now();
                self.send_ping().await?;
                self.state = SessionState::ChannelSetup;
            }
            ControlEvent::ChannelOpenRequest { channel_id: _ } => {
                // Rare: phone requested a channel open ON the control channel itself. Per aasdk,
                // the ack always goes back on ch=0 regardless of which channel was named.
                let mut outbox: Vec<(u8, u8, u16, Vec<u8>)> = Vec::new();
                let mut send = |c: u8, f: u8, m: u16, d: &[u8]| outbox.push((c, f, m, d.to_vec()));
                ControlChannel::send_channel_open_response(STATUS_OK, &mut send);
                for (c, f, m, d) in outbox {
                    self.send_aa(c, f, m, &d).await?;
                }
            }
            ControlEvent::Pong => {
                self.last_pong_at = Instant::now();
            }
            ControlEvent::VoiceSession(_active) => {
                // No consumer yet (aaDriver orchestration not built).
            }
            ControlEvent::Shutdown { reason } => {
                println!("[Session] Phone shutdown, reason={reason}");
                self.state = SessionState::Closed;
            }
            ControlEvent::ShutdownComplete => {
                // No HU-initiated shutdown flow built yet.
            }
            ControlEvent::None => {}
        }
        Ok(())
    }

    async fn handle_av_setup_request(
        &mut self,
        channel_id: u8,
        payload: &[u8],
        events: &mpsc::UnboundedSender<SessionEvent>,
    ) -> std::io::Result<()> {
        let req = AvChannelSetupRequest::decode(payload).unwrap_or(AvChannelSetupRequest {
            media_codec_type: media_codec::VIDEO_H264_BP,
        });

        if let Some(audio) = self.audio.get_mut(&channel_id) {
            let (rate, channels) = if channel_id == ch::MEDIA_AUDIO {
                (48000, 2)
            } else {
                (16000, 1)
            };
            audio.handle_setup_request(rate, channels);
        } else if channel_id == ch::MIC_INPUT {
            self.mic.handle_setup_request(16000, 1);
        }

        // Pick the config index matching whichever codec the phone asked for; fall back to
        // config 0 (always H264) if it asked for something we didn't offer.
        let mut config_idx: u32 = 0;
        if channel_id == ch::VIDEO || channel_id == ch::CLUSTER_VIDEO {
            let want = match req.media_codec_type {
                c if c == media_codec::VIDEO_H265 => VideoCodec::H265,
                c if c == media_codec::VIDEO_VP9 => VideoCodec::Vp9,
                c if c == media_codec::VIDEO_AV1 => VideoCodec::Av1,
                _ => VideoCodec::H264,
            };
            let offered = if channel_id == ch::VIDEO {
                &self.video_codec_by_index
            } else {
                &self.cluster_codec_by_index
            };
            if let Some(idx) = offered.iter().position(|c| *c == want) {
                config_idx = idx as u32;
            }
            let chosen = offered
                .get(config_idx as usize)
                .copied()
                .unwrap_or(VideoCodec::H264);
            if channel_id == ch::VIDEO {
                self.video_codec = Some(chosen);
            } else {
                self.cluster_codec = Some(chosen);
            }
        }

        let resp = AvChannelSetupResponse {
            media_status: av_setup_status::OK,
            max_unacked: Some(1),
            configs: vec![config_idx],
        };
        self.send_aa(
            channel_id,
            frame_flags::ENC_SIGNAL,
            av_msg::SETUP_RESPONSE,
            &resp.encode_to_vec(),
        )
        .await?;

        if channel_id == ch::VIDEO {
            // VideoFocusIndication(PROJECTED, unsolicited=false) — keyframe request.
            self.send_aa(
                ch::VIDEO,
                frame_flags::ENC_SIGNAL,
                av_msg::VIDEO_FOCUS_INDICATION,
                &[0x08, 0x01],
            )
            .await?;
            // No AVChannelStartIndication — phone sends START_INDICATION when ready.
            self.state = SessionState::Running;
            let _ = events.send(SessionEvent::Connected);
        }
        Ok(())
    }

    // ── Outbound helpers ──────────────────────────────────────────────────────

    async fn send_version_request(&mut self) -> std::io::Result<()> {
        let mut data = Vec::with_capacity(4);
        data.extend_from_slice(&version::MAJOR.to_be_bytes());
        data.extend_from_slice(&version::MINOR.to_be_bytes());
        let frame = encode_frame(
            ch::CONTROL,
            frame_flags::PLAINTEXT,
            ctrl_msg::VERSION_REQUEST,
            &data,
        );
        self.socket.write_all(&frame).await
    }

    async fn send_ping(&mut self) -> std::io::Result<()> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as i64;
        let buf = PingRequest {
            timestamp: ts,
            bug_report: None,
            data: None,
        }
        .encode_to_vec();
        self.send_aa(
            ch::CONTROL,
            frame_flags::PLAINTEXT,
            ctrl_msg::PING_REQUEST,
            &buf,
        )
        .await
    }

    /// Send an AA frame. Encrypted (flags & 0x08) frames are wrapped as one TLS record via the
    /// TLS engine first; plaintext frames go straight to the socket.
    async fn send_aa(
        &mut self,
        channel_id: u8,
        flags: u8,
        msg_id: u16,
        data: &[u8],
    ) -> std::io::Result<()> {
        let is_encrypted = flags & 0x08 != 0;
        if !is_encrypted {
            let frame = encode_frame(channel_id, flags, msg_id, data);
            return self.socket.write_all(&frame).await;
        }

        let Some(tls) = self.tls.as_mut() else {
            return Ok(());
        };
        let mut cleartext = Vec::with_capacity(2 + data.len());
        cleartext.extend_from_slice(&msg_id.to_be_bytes());
        cleartext.extend_from_slice(data);
        let ciphertext = tls.encrypt(&cleartext)?;
        if ciphertext.is_empty() {
            return Ok(());
        }

        let mut frame = Vec::with_capacity(4 + ciphertext.len());
        frame.push(channel_id);
        frame.push(flags);
        frame.extend_from_slice(&(ciphertext.len() as u16).to_be_bytes());
        frame.extend_from_slice(&ciphertext);
        self.socket.write_all(&frame).await
    }

    // ── Public outbound API (HU -> Phone) ─────────────────────────────────────

    /// Touch event in advertised touchscreen-space pixels. No-op outside `Running`.
    pub async fn send_touch(
        &mut self,
        action: u32,
        pointers: &[TouchPointer],
        action_index: u32,
    ) -> std::io::Result<()> {
        if self.state != SessionState::Running {
            return Ok(());
        }
        let Some(buf) = input_channel::build_touch_report(action, pointers, action_index) else {
            return Ok(());
        };
        self.send_aa(
            ch::INPUT,
            frame_flags::ENC_SIGNAL,
            input_channel::input_msg::INPUT_REPORT,
            &buf,
        )
        .await
    }

    /// HW button/key event. `key_codes` from `input_channel::button_key::*`.
    pub async fn send_button(
        &mut self,
        key_codes: &[u32],
        down: bool,
        longpress: bool,
    ) -> std::io::Result<()> {
        if self.state != SessionState::Running {
            return Ok(());
        }
        let Some(buf) = input_channel::build_button_report(key_codes, down, longpress) else {
            return Ok(());
        };
        self.send_aa(
            ch::INPUT,
            frame_flags::ENC_SIGNAL,
            input_channel::input_msg::INPUT_REPORT,
            &buf,
        )
        .await
    }

    /// Rotary-encoder delta event (-1 = previous, +1 = next).
    pub async fn send_rotary(&mut self, direction: i32) -> std::io::Result<()> {
        if self.state != SessionState::Running {
            return Ok(());
        }
        let buf = input_channel::build_rotary_report(direction);
        self.send_aa(
            ch::INPUT,
            frame_flags::ENC_SIGNAL,
            input_channel::input_msg::INPUT_REPORT,
            &buf,
        )
        .await
    }

    /// Push captured mic PCM (s16le, matching the rate/channels negotiated in
    /// `AvChannelSetupRequest`, typically 16 kHz mono) to the phone. No-op outside `Running` or
    /// while the phone hasn't opened the mic (see `MicChannel::push_pcm`).
    pub async fn push_mic_pcm(&mut self, data: Vec<u8>, timestamp_ns: u64) -> std::io::Result<()> {
        if self.state != SessionState::Running {
            return Ok(());
        }
        let mut outbox: Vec<(u8, u8, u16, Vec<u8>)> = Vec::new();
        let mut send = |c: u8, f: u8, m: u16, d: &[u8]| outbox.push((c, f, m, d.to_vec()));
        self.mic.push_pcm(data, timestamp_ns, &mut send);
        for (c, f, m, d) in outbox {
            self.send_aa(c, f, m, &d).await?;
        }
        Ok(())
    }

    /// Ask the phone to switch the main display back to projected AA content (mode=PROJECTED,
    /// reason=UNKNOWN) — used to resume after the phone requested the host UI (see
    /// `SessionEvent::HostUiRequested`). No-op outside `Running`.
    pub async fn request_video_focus(&mut self) -> std::io::Result<()> {
        if self.state != SessionState::Running {
            println!(
                "[Session] request_video_focus: no-op, state={:?} (not Running)",
                self.state
            );
            return Ok(());
        }
        let result = self
            .send_aa(
                ch::VIDEO,
                frame_flags::ENC_SIGNAL,
                av_msg::VIDEO_FOCUS_REQUEST,
                &[0x10, 0x01, 0x18, 0x00],
            )
            .await;
        println!("[Session] request_video_focus: sent VIDEO_FOCUS_REQUEST mode=PROJECTED, result={result:?}");
        result
    }
}

fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}
