//! Builds the ServiceDiscoveryResponse (SDR) — the HU's advertisement of every channel
//! (video/audio/input/sensor/bluetooth/navigation/...) it offers to the phone, sent in reply to
//! the phone's ServiceDiscoveryRequest.

use aa_proto::aap_protobuf::service::bluetooth::BluetoothService;
use aa_proto::aap_protobuf::service::control::message::{
    ConnectionConfiguration, HeadUnitInfo, PingConfiguration, ServiceDiscoveryResponse,
};
use aa_proto::aap_protobuf::service::inputsource::{input_source_service, InputSourceService};
use aa_proto::aap_protobuf::service::media::shared::message::{
    AudioConfiguration, Insets, UiConfig,
};
use aa_proto::aap_protobuf::service::media::sink::{message as sink_message, MediaSinkService};
use aa_proto::aap_protobuf::service::media::source::MediaSourceService;
use aa_proto::aap_protobuf::service::mediaplayback::MediaPlaybackStatusService;
use aa_proto::aap_protobuf::service::navigationstatus::{
    navigation_status_service, NavigationStatusService,
};
use aa_proto::aap_protobuf::service::phonestatus::PhoneStatusService;
use aa_proto::aap_protobuf::service::radio::message::{RadioProperties, RadioType, Range};
use aa_proto::aap_protobuf::service::radio::RadioService;
use aa_proto::aap_protobuf::service::sensorsource::{
    message as sensor_message, SensorSourceService,
};
use aa_proto::aap_protobuf::service::wifiprojection::WifiProjectionService;
use aa_proto::aap_protobuf::service::Service;
use prost::Message;

use super::super::constants::{
    bt_pairing_method, ch, display_type, media_codec, sensor_type, video_fps, video_resolution,
};
use super::config::{SessionConfig, VideoCodec};

pub struct ServiceDiscoveryResult {
    pub buf: Vec<u8>,
    pub video_codec_by_index: Vec<VideoCodec>,
    pub cluster_codec_by_index: Vec<VideoCodec>,
    // The negotiated video tier (encoded frame size, including whatever letterbox bars the phone
    // pads in to match our advertised display aspect) and the actual AA content region within
    // it — crop this out and scale the remainder up to fill the real screen, or the video just
    // renders at the tier's native size (see GstVideo::set_content_region).
    pub video_tier_width: u32,
    pub video_tier_height: u32,
    pub video_crop_left: u32,
    pub video_crop_top: u32,
    pub video_vis_width: u32,
    pub video_vis_height: u32,
    // The touchscreen space actually advertised in the INPUT channel below (tier minus both the
    // AR-letterbox margin and any user view-area inset) — touch coordinates sent to the phone
    // must be scaled against this, not `video_vis_width`/`height`, whenever a view-area inset
    // is configured (see `commands::aa_send_pointer`/`aa_send_touch`).
    pub touch_width: u32,
    pub touch_height: u32,
}

fn resolution_from_dimensions(w: u32, h: u32) -> Option<i32> {
    match (w, h) {
        (800, 480) => Some(video_resolution::R800X480),
        (1280, 720) => Some(video_resolution::R1280X720),
        (1920, 1080) => Some(video_resolution::R1920X1080),
        _ => None,
    }
}

struct Margins {
    top: u32,
    bottom: u32,
    left: u32,
    right: u32,
}

fn letterbox_margins(content_w: u32, content_h: u32, tier_w: u32, tier_h: u32) -> (u32, u32) {
    if content_w == 0 || content_h == 0 || tier_w == 0 || tier_h == 0 {
        return (0, 0);
    }
    let content_ar = content_w as f64 / content_h as f64;
    let tier_ar = tier_w as f64 / tier_h as f64;
    if content_ar > tier_ar {
        let content_h_fit = ((tier_w as f64 / content_ar).round() as i64 as u32) & !1;
        (0, tier_h.saturating_sub(content_h_fit))
    } else if content_ar < tier_ar {
        let content_w_fit = ((tier_h as f64 * content_ar).round() as i64 as u32) & !1;
        (tier_w.saturating_sub(content_w_fit), 0)
    } else {
        (0, 0)
    }
}

pub fn build_service_discovery_response(cfg: &SessionConfig) -> ServiceDiscoveryResult {
    let v_w = cfg.video_width.unwrap_or(1280);
    let v_h = cfg.video_height.unwrap_or(720);
    let dpi = cfg.video_dpi.unwrap_or(140);
    let fps = cfg.video_fps.unwrap_or(30);

    // VideoCodecResolutionType: 800x480=1, 1280x720=2, 1920x1080=3, 2560x1440=4, 3840x2160=5
    let v_res: i32 = if v_w >= 3840 {
        5
    } else if v_w >= 2560 {
        4
    } else if v_w >= 1920 {
        video_resolution::R1920X1080
    } else if v_w <= 800 {
        video_resolution::R800X480
    } else {
        video_resolution::R1280X720
    };
    let v_fps = if fps == 60 {
        video_fps::FPS60
    } else {
        video_fps::FPS30
    };

    let (width_margin, height_margin) =
        if cfg.display_width.unwrap_or(0) > 0 && cfg.display_height.unwrap_or(0) > 0 {
            letterbox_margins(
                cfg.display_width.unwrap(),
                cfg.display_height.unwrap(),
                v_w,
                v_h,
            )
        } else {
            (0, 0)
        };

    // View Area -> margins (AR letterbox + user view inset). Safe Area -> content_insets.
    let view_top = cfg.main_view_area_top.unwrap_or(0);
    let view_bottom = cfg.main_view_area_bottom.unwrap_or(0);
    let view_left = cfg.main_view_area_left.unwrap_or(0);
    let view_right = cfg.main_view_area_right.unwrap_or(0);
    let inset_top = height_margin / 2 + view_top;
    let inset_bottom = (height_margin - height_margin / 2) + view_bottom;
    let inset_left = width_margin / 2 + view_left;
    let inset_right = (width_margin - width_margin / 2) + view_right;
    let main_content_insets = Insets {
        top: Some(cfg.main_safe_area_top.unwrap_or(0)),
        bottom: Some(cfg.main_safe_area_bottom.unwrap_or(0)),
        left: Some(cfg.main_safe_area_left.unwrap_or(0)),
        right: Some(cfg.main_safe_area_right.unwrap_or(0)),
    };

    // AudioStreamType: GUIDANCE=1, SYSTEM=2, MEDIA=3
    const AS_GUIDANCE: i32 = 1;
    const AS_SYSTEM: i32 = 2;
    const AS_MEDIA: i32 = 3;

    let mut channels: Vec<Service> = Vec::new();

    // -- Video (ch=3) --
    let par_e4 = cfg.pixel_aspect_ratio_e4.unwrap_or(10000);
    let video_ui_config = UiConfig {
        margins: Some(Insets {
            top: Some(inset_top),
            bottom: Some(inset_bottom),
            left: Some(inset_left),
            right: Some(inset_right),
        }),
        content_insets: Some(main_content_insets),
        stable_content_insets: Some(main_content_insets),
        ui_theme: None,
    };
    let base_video_config = sink_message::VideoConfiguration {
        codec_resolution: Some(v_res),
        frame_rate: Some(v_fps),
        width_margin: Some(width_margin),
        height_margin: Some(height_margin),
        density: Some(dpi),
        decoder_additional_depth: None,
        viewing_distance: None,
        pixel_aspect_ratio_e4: Some(par_e4),
        real_density: None,
        video_codec_type: None,
        ui_config: Some(video_ui_config),
    };
    let mut video_configs = vec![sink_message::VideoConfiguration {
        video_codec_type: Some(media_codec::VIDEO_H264_BP),
        ..base_video_config
    }];
    let mut video_codec_by_index = vec![VideoCodec::H264];
    if cfg.hevc_supported {
        video_configs.push(sink_message::VideoConfiguration {
            video_codec_type: Some(media_codec::VIDEO_H265),
            ..base_video_config
        });
        video_codec_by_index.push(VideoCodec::H265);
    }
    if cfg.vp9_supported {
        video_configs.push(sink_message::VideoConfiguration {
            video_codec_type: Some(media_codec::VIDEO_VP9),
            ..base_video_config
        });
        video_codec_by_index.push(VideoCodec::Vp9);
    }
    if cfg.av1_supported {
        video_configs.push(sink_message::VideoConfiguration {
            video_codec_type: Some(media_codec::VIDEO_AV1),
            ..base_video_config
        });
        video_codec_by_index.push(VideoCodec::Av1);
    }
    channels.push(Service {
        id: ch::VIDEO as i32,
        media_sink_service: Some(MediaSinkService {
            available_type: Some(media_codec::VIDEO_H264_BP),
            available_while_in_call: Some(true),
            video_configs,
            ..Default::default()
        }),
        ..Default::default()
    });

    // -- Cluster Video (ch=19) — secondary display sink for Maps/Navi --
    let mut cluster_codec_by_index: Vec<VideoCodec> = Vec::new();
    if cfg.cluster_enabled {
        let c_w = cfg.cluster_width;
        let c_h = cfg.cluster_height;
        let c_tier_w = cfg.cluster_tier_width.unwrap_or(c_w);
        let c_tier_h = cfg.cluster_tier_height.unwrap_or(c_h);
        let cluster_res = resolution_from_dimensions(c_tier_w, c_tier_h).unwrap_or(v_res);
        let cluster_fps = if cfg.cluster_fps == 60 {
            video_fps::FPS60
        } else if cfg.cluster_fps == 30 {
            video_fps::FPS30
        } else {
            v_fps
        };
        let cluster_dpi = cfg.cluster_dpi;

        let (c_w_margin, c_h_margin) = letterbox_margins(c_w, c_h, c_tier_w, c_tier_h);

        let cluster_view_top = cfg.cluster_view_area_top.unwrap_or(0);
        let cluster_view_bottom = cfg.cluster_view_area_bottom.unwrap_or(0);
        let cluster_view_left = cfg.cluster_view_area_left.unwrap_or(0);
        let cluster_view_right = cfg.cluster_view_area_right.unwrap_or(0);
        let cluster_margins = Margins {
            top: c_h_margin / 2 + cluster_view_top,
            bottom: (c_h_margin - c_h_margin / 2) + cluster_view_bottom,
            left: c_w_margin / 2 + cluster_view_left,
            right: (c_w_margin - c_w_margin / 2) + cluster_view_right,
        };
        let cluster_content_insets = Insets {
            top: Some(cfg.cluster_safe_area_top.unwrap_or(0)),
            bottom: Some(cfg.cluster_safe_area_bottom.unwrap_or(0)),
            left: Some(cfg.cluster_safe_area_left.unwrap_or(0)),
            right: Some(cfg.cluster_safe_area_right.unwrap_or(0)),
        };

        let cluster_base = sink_message::VideoConfiguration {
            codec_resolution: Some(cluster_res),
            frame_rate: Some(cluster_fps),
            width_margin: Some(c_w_margin),
            height_margin: Some(c_h_margin),
            density: Some(cluster_dpi),
            decoder_additional_depth: None,
            viewing_distance: None,
            pixel_aspect_ratio_e4: Some(cfg.cluster_pixel_aspect_ratio_e4.unwrap_or(10000)),
            real_density: None,
            video_codec_type: None,
            ui_config: Some(UiConfig {
                margins: Some(Insets {
                    top: Some(cluster_margins.top),
                    bottom: Some(cluster_margins.bottom),
                    left: Some(cluster_margins.left),
                    right: Some(cluster_margins.right),
                }),
                content_insets: Some(cluster_content_insets),
                stable_content_insets: Some(cluster_content_insets),
                ui_theme: None,
            }),
        };
        let mut cluster_configs = vec![sink_message::VideoConfiguration {
            video_codec_type: Some(media_codec::VIDEO_H264_BP),
            ..cluster_base
        }];
        cluster_codec_by_index.push(VideoCodec::H264);
        if cfg.hevc_supported {
            cluster_configs.push(sink_message::VideoConfiguration {
                video_codec_type: Some(media_codec::VIDEO_H265),
                ..cluster_base
            });
            cluster_codec_by_index.push(VideoCodec::H265);
        }
        if cfg.vp9_supported {
            cluster_configs.push(sink_message::VideoConfiguration {
                video_codec_type: Some(media_codec::VIDEO_VP9),
                ..cluster_base
            });
            cluster_codec_by_index.push(VideoCodec::Vp9);
        }
        if cfg.av1_supported {
            cluster_configs.push(sink_message::VideoConfiguration {
                video_codec_type: Some(media_codec::VIDEO_AV1),
                ..cluster_base
            });
            cluster_codec_by_index.push(VideoCodec::Av1);
        }
        channels.push(Service {
            id: ch::CLUSTER_VIDEO as i32,
            media_sink_service: Some(MediaSinkService {
                available_type: Some(media_codec::VIDEO_H264_BP),
                available_while_in_call: Some(true),
                video_configs: cluster_configs,
                display_type: Some(display_type::CLUSTER),
                display_id: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        });
        channels.push(Service {
            id: ch::CLUSTER_INPUT as i32,
            input_source_service: Some(InputSourceService {
                display_id: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        });
    }

    // -- Audio sinks + Microphone --
    if !cfg.disable_audio_output {
        channels.push(Service {
            id: ch::MEDIA_AUDIO as i32,
            media_sink_service: Some(MediaSinkService {
                available_type: Some(media_codec::AUDIO_PCM),
                audio_type: Some(AS_MEDIA),
                available_while_in_call: Some(true),
                audio_configs: vec![AudioConfiguration {
                    sampling_rate: 48000,
                    number_of_bits: 16,
                    number_of_channels: 2,
                }],
                ..Default::default()
            }),
            ..Default::default()
        });
        channels.push(Service {
            id: ch::SPEECH_AUDIO as i32,
            media_sink_service: Some(MediaSinkService {
                available_type: Some(media_codec::AUDIO_PCM),
                audio_type: Some(AS_GUIDANCE),
                available_while_in_call: Some(true),
                audio_configs: vec![AudioConfiguration {
                    sampling_rate: 16000,
                    number_of_bits: 16,
                    number_of_channels: 1,
                }],
                ..Default::default()
            }),
            ..Default::default()
        });
    }

    channels.push(Service {
        id: ch::SYSTEM_AUDIO as i32,
        media_sink_service: Some(MediaSinkService {
            available_type: Some(media_codec::AUDIO_PCM),
            audio_type: Some(AS_SYSTEM),
            available_while_in_call: Some(true),
            audio_configs: vec![AudioConfiguration {
                sampling_rate: 16000,
                number_of_bits: 16,
                number_of_channels: 1,
            }],
            ..Default::default()
        }),
        ..Default::default()
    });

    channels.push(Service {
        id: ch::MIC_INPUT as i32,
        media_source_service: Some(MediaSourceService {
            available_type: Some(media_codec::AUDIO_PCM),
            audio_config: Some(AudioConfiguration {
                sampling_rate: 16000,
                number_of_bits: 16,
                number_of_channels: 1,
            }),
            available_while_in_call: Some(true),
        }),
        ..Default::default()
    });

    // channels.push(Service {
    //     id: ch::RADIO as i32,
    //     radio_service: Some(RadioService {
    //         radio_properties: vec![RadioProperties {
    //             radio_id: 1,
    //             r#type: RadioType::FmRadio as i32,
    //             channel_range: vec![Range {
    //                 min: 87500,
    //                 max: 108000,
    //             }],
    //             channel_spacing: 100,
    //             ..Default::default()
    //         }],
    //         ..Default::default()
    //     }),
    //     ..Default::default()
    // });

    // -- Sensor Source (ch=1) --
    let fuel_types = if cfg.fuel_types.is_empty() {
        vec![1]
    } else {
        cfg.fuel_types.clone()
    };
    let sensor = |t: i32| sensor_message::Sensor { sensor_type: t };
    channels.push(Service {
        id: ch::SENSOR as i32,
        sensor_source_service: Some(SensorSourceService {
            sensors: vec![
                sensor(sensor_type::DRIVING_STATUS),
                sensor(sensor_type::GPS_LOCATION),
                sensor(sensor_type::NIGHT_DATA),
                sensor(sensor_type::CAR_SPEED),
                sensor(8), // GEAR
                sensor(sensor_type::PARKING_BRAKE),
                sensor(6),  // FUEL
                sensor(5),  // ODOMETER
                sensor(11), // ENV_DATA
                sensor(16), // DOOR_DATA
                sensor(17), // LIGHT_DATA
                sensor(18), // TIRE_PRESSURE_DATA
                sensor(12), // HVAC
                sensor(19), // ACCELEROMETER
                sensor(20), // GYROSCOPE
                sensor(2),  // COMPASS
                sensor(21), // GPS_SATELLITE
                sensor(sensor_type::RPM),
                sensor(23), // VEHICLE_ENERGY_MODEL
                sensor(25), // RAW_VEHICLE_ENERGY_MODEL
                sensor(26), // RAW_EV_TRIP_SETTINGS
            ],
            // RAW_GPS_ONLY=256 | ACCEL=4 | GYRO=2 | COMPASS=8 | CAR_SPEED=64
            location_characterization: Some(256 | 4 | 2 | 8 | 64),
            supported_fuel_types: fuel_types,
            supported_ev_connector_types: cfg.ev_connector_types.clone(),
        }),
        ..Default::default()
    });

    // -- Input Source (ch=8) --
    let touch_w = (v_w.saturating_sub(inset_left).saturating_sub(inset_right)).max(1) as i32;
    let touch_h = (v_h.saturating_sub(inset_top).saturating_sub(inset_bottom)).max(1) as i32;
    channels.push(Service {
        id: ch::INPUT as i32,
        input_source_service: Some(InputSourceService {
            keycodes_supported: vec![
                3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
                25, 26, 66, 79, 82, 84, 85, 86, 87, 88, 89, 90, 91, 111, 126, 127, 164, 219, 231,
                260, 261, 262, 263, 65536,
            ],
            touchscreen: vec![input_source_service::TouchScreen {
                width: touch_w,
                height: touch_h,
                r#type: None,
                is_secondary: None,
            }],
            ..Default::default()
        }),
        ..Default::default()
    });

    // -- Bluetooth (ch=10) --
    channels.push(Service {
        id: ch::BLUETOOTH as i32,
        bluetooth_service: Some(BluetoothService {
            car_address: cfg
                .bt_mac_address
                .clone()
                .unwrap_or_else(|| "00:00:00:00:00:00".to_string()),
            supported_pairing_methods: vec![
                bt_pairing_method::PIN,
                bt_pairing_method::NUMERIC_COMPARISON,
            ],
        }),
        ..Default::default()
    });

    // -- Navigation Status (ch=12) --
    channels.push(Service {
        id: ch::NAVIGATION as i32,
        navigation_status_service: Some(NavigationStatusService {
            minimum_interval_ms: 500,
            r#type: 1,
            image_options: Some(navigation_status_service::ImageOptions {
                width: 256,
                height: 256,
                colour_depth_bits: 32,
            }),
        }),
        ..Default::default()
    });

    channels.push(Service {
        id: ch::MEDIA_INFO as i32,
        media_playback_service: Some(MediaPlaybackStatusService {}),
        ..Default::default()
    });
    channels.push(Service {
        id: ch::PHONE_STATUS as i32,
        phone_status_service: Some(PhoneStatusService {}),
        ..Default::default()
    });

    if let Some(bssid) = cfg.wifi_bssid.clone() {
        channels.push(Service {
            id: ch::WIFI as i32,
            wifi_projection_service: Some(WifiProjectionService {
                car_wifi_bssid: Some(bssid),
            }),
            ..Default::default()
        });
    }

    #[allow(deprecated)]
    let sdr = ServiceDiscoveryResponse {
        channels,
        make: Some("AVIO".to_string()),
        model: Some("Universal".to_string()),
        year: Some("2026".to_string()),
        vehicle_id: Some("avio-001".to_string()),
        driver_position: Some(cfg.driver_position as i32),
        head_unit_make: Some("AVIO".to_string()),
        head_unit_model: Some("AVIO Head Unit".to_string()),
        head_unit_software_build: Some("1".to_string()),
        head_unit_software_version: Some("1.0".to_string()),
        can_play_native_media_during_vr: Some(true),
        session_configuration: None,
        display_name: Some(cfg.hu_name.clone().unwrap_or_else(|| "AVIO".to_string())),
        probe_for_support: Some(false),
        connection_configuration: Some(ConnectionConfiguration {
            ping_configuration: Some(PingConfiguration {
                timeout_ms: Some(5000),
                interval_ms: Some(1500),
                high_latency_threshold_ms: Some(500),
                tracked_ping_count: Some(5),
            }),
            wireless_tcp_configuration: None,
        }),
        headunit_info: Some(HeadUnitInfo {
            make: Some("AVIO".to_string()),
            model: Some("Universal".to_string()),
            year: Some("2026".to_string()),
            vehicle_id: Some("avio-001".to_string()),
            head_unit_make: Some("AVIO".to_string()),
            head_unit_model: Some("AVIO Head Unit".to_string()),
            head_unit_software_build: Some("1".to_string()),
            head_unit_software_version: Some("1.0".to_string()),
        }),
    };

    let buf = sdr.encode_to_vec();

    ServiceDiscoveryResult {
        buf,
        video_codec_by_index,
        cluster_codec_by_index,
        video_tier_width: v_w,
        video_tier_height: v_h,
        video_crop_left: width_margin / 2,
        video_crop_top: height_margin / 2,
        video_vis_width: v_w.saturating_sub(width_margin),
        video_vis_height: v_h.saturating_sub(height_margin),
        touch_width: touch_w as u32,
        touch_height: touch_h as u32,
    }
}
