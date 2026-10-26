//! Wires a USB-attached phone into the Android Auto stack: runs the AOAP handshake (via
//! `UsbAoapBridge`), dials the resulting USB<->TCP loopback, and drives an AA `Session` over it.
//! Video frames get decoded and rendered via the `gst_video` pipeline; audio channels are played
//! out via `AudioOutput` (one `gst-launch-1.0` PCM sink per channel).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::audio::audio_output::{AudioOutput, AudioOutputOptions};
use crate::store::{settings_store, status_store};
use crate::video::gst_video::{probe_gst_codecs, GstVideo, GstVideoCodec, VideoRuntime};

use super::session_handle::AaSessionHandle;
use super::stack::aoap::constants::{AOAP_LOOPBACK_HOST, AOAP_LOOPBACK_PORT};
use super::stack::constants::ch;
use super::stack::session::config::{SessionConfig, VideoCodec};
use super::stack::session::session::{Session, SessionEvent};
use super::stack::transport::usb_aoap_bridge::{BridgeEvent, UsbAoapBridge};
use super::video_tier;

/// Matches the rates `Session::handle_av_setup_request` negotiates per channel.
fn audio_format_for(channel_id: u8) -> (i32, i32) {
    if channel_id == ch::MEDIA_AUDIO {
        (48000, 2)
    } else {
        (16000, 1)
    }
}

fn to_gst_codec(codec: VideoCodec) -> GstVideoCodec {
    match codec {
        VideoCodec::H264 => GstVideoCodec::H264,
        VideoCodec::H265 => GstVideoCodec::H265,
        VideoCodec::Vp9 => GstVideoCodec::VP9,
        VideoCodec::Av1 => GstVideoCodec::AV1,
    }
}

/// The main window's actual physical size, now that it's resized to fill the monitor at startup.
/// Fed into `SessionConfig::display_width/height` so the SDR computes real letterbox
/// margins for our screen's aspect ratio — without it, the phone assumes its 16:9
/// `video_width`/`video_height` tier exactly matches the display, which stretches AA's video
/// into whatever (differently-shaped) window we actually give it.
fn display_size() -> Option<(u32, u32)> {
    status_store::display_size()
}

pub async fn connect_wired(phone: nusb::DeviceInfo) {
    let handle = crate::projection::driver::aa::session_handle::shared();
    let current_settings = settings_store::snapshot();

    let bridge = UsbAoapBridge::new();
    let (bridge_tx, mut bridge_rx) = mpsc::unbounded_channel();

    if let Err(e) = bridge.start(phone, AOAP_LOOPBACK_PORT, bridge_tx).await {
        eprintln!("[AA wired] failed to start USB<->TCP bridge: {e}");
        return;
    }

    // Wait for the bridge's loopback listener to come up, then dial it and hand the socket to
    // a Session. The bridge must be explicitly stopped once we're done with it — dropping the
    // handle does not cancel its background task (a JoinHandle drop just detaches it), which
    // would otherwise leave its TcpListener on the loopback port bound forever, breaking the
    // next connect attempt with "Address already in use".
    'bridge_wait: {
        let event = tokio::select! {
            event = bridge_rx.recv() => event,
            () = handle.shutdown_notify().notified() => break 'bridge_wait,
        };
        let Some(event) = event else {
            break 'bridge_wait;
        };

        match event {
            BridgeEvent::Ready { host, port } => {
                println!("[AA wired] USB bridge ready on {host}:{port}, dialing loopback");
                match TcpStream::connect((AOAP_LOOPBACK_HOST, AOAP_LOOPBACK_PORT)).await {
                    Ok(socket) => {
                        let probe = probe_gst_codecs();
                        let screen = display_size();
                        let (display_width, display_height) = screen.unzip();
                        let tier = video_tier::resolve(
                            &current_settings.ao_resolution,
                            current_settings.ao_dpi_auto,
                            current_settings.ao_dpi,
                            screen,
                        );
                        println!(
                            "[AA wired] video {}x{} at {} dpi (resolution {}, dpi {}, screen {:?})",
                            tier.width,
                            tier.height,
                            tier.dpi,
                            current_settings.ao_resolution,
                            if current_settings.ao_dpi_auto { "auto" } else { "manual" },
                            screen,
                        );
                        let cfg = SessionConfig {
                            hevc_supported: probe.h265.hw || probe.h265.sw,
                            vp9_supported: probe.vp9.hw || probe.vp9.sw,
                            av1_supported: probe.av1.hw || probe.av1.sw,
                            display_width,
                            display_height,
                            video_width: Some(tier.width),
                            video_height: Some(tier.height),
                            video_dpi: Some(tier.dpi),
                            video_fps: current_settings.ao_framerate.parse().ok(),
                            main_view_area_top: Some(current_settings.ao_view_area_top),
                            main_view_area_bottom: Some(current_settings.ao_view_area_bottom),
                            main_view_area_left: Some(current_settings.ao_view_area_left),
                            main_view_area_right: Some(current_settings.ao_view_area_right),
                            main_safe_area_top: Some(current_settings.ao_safe_area_top),
                            main_safe_area_bottom: Some(current_settings.ao_safe_area_bottom),
                            main_safe_area_left: Some(current_settings.ao_safe_area_left),
                            main_safe_area_right: Some(current_settings.ao_safe_area_right),
                            ..SessionConfig::default()
                        };
                        let session = Session::new(socket, cfg);
                        let (session_tx, session_rx) = mpsc::unbounded_channel();
                        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
                        handle.set(cmd_tx).await;
                        tokio::spawn(handle_session_events(
                            handle.clone(),
                            session_rx,
                        ));
                        session
                            .run(session_tx, cmd_rx, handle.shutdown_notify())
                            .await;
                        handle.clear().await;
                        println!("[AA wired] session ended");
                    }
                    Err(e) => eprintln!("[AA wired] failed to dial loopback: {e}"),
                }
            }
            BridgeEvent::Error(e) => {
                eprintln!("[AA wired] USB bridge error: {e}");
            }
            BridgeEvent::Closed => {}
        }
    }

    bridge.stop().await;
}

/// Consumes session events: decodes+renders the main display's video, logs everything else.
async fn handle_session_events(
    handle: Arc<AaSessionHandle>,
    mut rx: mpsc::UnboundedReceiver<SessionEvent>,
) {
    let mut video: Option<GstVideo> = None;
    // Tracks whether the video plane is currently shown, so it can be hidden while the phone is
    // showing its own host UI (otherwise the last projected frame just freezes there, visible)
    // and shown again once fresh frames resume.
    let mut video_visible = true;
    // One AudioOutput per active audio channel (MEDIA/SPEECH/SYSTEM can all be live at once).
    let mut audio: HashMap<u8, AudioOutput> = HashMap::new();
    // Crop-out-letterbox-bars-and-scale-to-fill region from the SDR negotiation, applied once
    // "video" exists. Arrives well before the first frame (sent during service discovery), so
    // it's normally applied right at video creation below — stashed here in case it ever arrives
    // late instead.
    let mut pending_geometry: Option<(f64, f64, f64, f64, f64, f64)> = None;
    // Set once from the phone's ServiceDiscoveryRequest (arrives before "connected"), then reused
    // for every AAStatus emitted below — including the final "disconnected" one, so the UI can
    // still show which device just dropped.
    let mut device_name: Option<String> = None;

    while let Some(event) = rx.recv().await {
        match event {
            SessionEvent::DeviceInfo { device_name: name } => {
                device_name = name;
            }
            SessionEvent::MediaStatus(media_status) => {
                status_store::set_aa_media_status(media_status);
            }
            SessionEvent::MediaMetadata(media_metadata) => {
                status_store::set_aa_media_metadata(media_metadata);
            }
            SessionEvent::VideoGeometry {
                crop_left,
                crop_top,
                vis_width,
                vis_height,
                tier_width,
                tier_height,
                touch_width,
                touch_height,
            } => {
                handle.set_touch_size(touch_width, touch_height).await;
                let region = (
                    crop_left as f64,
                    crop_top as f64,
                    vis_width as f64,
                    vis_height as f64,
                    tier_width as f64,
                    tier_height as f64,
                );
                if let Some(v) = video.as_mut() {
                    v.set_content_region(
                        region.0, region.1, region.2, region.3, region.4, region.5,
                    )
                    .await;
                }
                pending_geometry = Some(region);
            }
            SessionEvent::VideoFrame {
                channel_id,
                codec,
                data,
                ..
            } => {
                if channel_id != ch::VIDEO {
                    continue; // cluster display not wired up yet
                }
                let is_first_frame = video.is_none();
                if is_first_frame {
                    // 0: on Linux the gst-host process makes its own toplevel, which the
                    // compositor places by tag (see GstVideo::native_handle)
                    let mut new_video =
                        GstVideo::new(VideoRuntime::shared(), 0, "android-auto", "main");
                    if let Some(region) = pending_geometry {
                        new_video
                            .set_content_region(
                                region.0, region.1, region.2, region.3, region.4, region.5,
                            )
                            .await;
                    }
                    video = Some(new_video);
                }
                if let Some(v) = video.as_mut() {
                    if !video_visible {
                        v.set_visible(true).await;
                        video_visible = true;
                        // SessionEvent::Connected only fires once, at initial connection - after
                        // a HostUiRequested detour set aaStatus to "host-ui" (see App.vue's
                        // androidAutoActive), nothing else ever flips it back to "connected", so
                        // the UI stayed stuck on the resume screen even once frames resumed.
                        status_store::set_aa_status("connected", device_name.as_deref());
                    }
                    v.push(to_gst_codec(codec), &data).await;
                }
                if is_first_frame {
                    // Creating the video player maps a new native surface (waylandsink, via
                    // gst-host), which now also requests fullscreen directly on Linux when no
                    // external compositor is present (see gst_video.cc) — a state transition
                    // that can steal focus while it settles. There's no signal for "surface
                    // mapped and fullscreen settled", so keep reclaiming focus on "main" (whose
                    // own transparent pointer-capture layer now handles touch, see App.vue) for
                    // longer than a single re-assertion would cover.
                    tokio::spawn(async move {
                        for _ in 0..30 {
                            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                            status_store::request_focus();
                        }
                    });
                }
            }
            SessionEvent::AudioStart { channel_id } => {
                let (sample_rate, channels) = audio_format_for(channel_id);
                let mut output = AudioOutput::new(AudioOutputOptions {
                    sample_rate,
                    channels,
                    mode: None,
                    device: None,
                });
                output.start().await;
                audio.insert(channel_id, output);
            }
            SessionEvent::AudioFrame {
                channel_id, data, ..
            } => {
                if let Some(output) = audio.get_mut(&channel_id) {
                    let samples: Vec<i16> = data
                        .as_chunks::<2>().0.iter()
                        .map(|b| i16::from_le_bytes([b[0], b[1]]))
                        .collect();
                    output.write(samples).await;
                }
            }
            SessionEvent::AudioStop { channel_id } => {
                if let Some(mut output) = audio.remove(&channel_id) {
                    output.stop().await;
                }
            }
            SessionEvent::Connected => {
                status_store::set_aa_status("connected", device_name.as_deref());
            }
            SessionEvent::HostUiRequested => {
                println!("[AA wired] HostUiRequested: hiding video, focusing main");
                // Otherwise the last projected frame just stays frozen on screen, visible
                // underneath the resume button, until the phone starts sending fresh frames.
                if let Some(v) = video.as_mut() {
                    v.set_visible(false).await;
                    video_visible = false;
                }
                // Hiding the (focused) touch overlay doesn't guarantee focus lands back on
                // "main" — without this, clicks on the resume button can land nowhere.
                status_store::request_focus();
                status_store::set_aa_status("host-ui", device_name.as_deref());
            }
            SessionEvent::Disconnected => {
                // Also emitted unconditionally after the loop below, which covers every exit
                // path (including the channel just closing); nothing further to do here.
            }
            other => {
                println!("[AA wired] session event: {other:?}");
            }
        }
    }

    status_store::set_aa_status("disconnected", device_name.as_deref());

    if let Some(mut v) = video {
        v.dispose().await;
    }
    for (_, mut output) in audio.drain() {
        output.stop().await;
    }
}
