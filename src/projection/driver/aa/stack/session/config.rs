//! Per-session configuration: HU identity, advertised video/display geometry, sensors,
//! Bluetooth/WiFi projection details. Mirrors AVIO's `Session.ts` `SessionConfig`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
    Vp9,
    Av1,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    // HU label in SDR
    pub hu_name: Option<String>,
    // AA tier the phone encodes into (800x480 / 1280x720 / 1920x1080 / 2560x1440 / 3840x2160)
    pub video_width: Option<u32>,
    pub video_height: Option<u32>,
    pub video_dpi: Option<u32>,
    pub video_fps: Option<u32>, // 30 | 60
    pub pixel_aspect_ratio_e4: Option<u32>,
    // Physical HU display
    pub display_width: Option<u32>,
    pub display_height: Option<u32>,
    // View Area -> margins, Safe Area -> content_insets
    pub main_view_area_top: Option<u32>,
    pub main_view_area_bottom: Option<u32>,
    pub main_view_area_left: Option<u32>,
    pub main_view_area_right: Option<u32>,
    pub main_safe_area_top: Option<u32>,
    pub main_safe_area_bottom: Option<u32>,
    pub main_safe_area_left: Option<u32>,
    pub main_safe_area_right: Option<u32>,
    // Driver seat position (LHD=0 / RHD=1)
    pub driver_position: u8,
    // BT adapter MAC for BT channel
    pub bt_mac_address: Option<String>,
    // WiFi AP BSSID/SSID/password/channel
    pub wifi_bssid: Option<String>,
    pub wifi_ssid: Option<String>,
    pub wifi_password: Option<String>,
    // FuelType (UNLEADED=1, DIESEL_2=4, ELECTRIC=10, ...)
    pub fuel_types: Vec<i32>,
    pub ev_connector_types: Vec<i32>,
    // Renderer probe results — only codecs flagged true are advertised
    pub hevc_supported: bool,
    pub vp9_supported: bool,
    pub av1_supported: bool,
    pub initial_night_mode: bool,
    // When true the secondary (CLUSTER) video sink is advertised in the SDR
    pub cluster_enabled: bool,
    pub cluster_width: u32,
    pub cluster_height: u32,
    pub cluster_tier_width: Option<u32>,
    pub cluster_tier_height: Option<u32>,
    pub cluster_pixel_aspect_ratio_e4: Option<u32>,
    pub cluster_fps: u32,
    pub cluster_dpi: u32,
    pub cluster_view_area_top: Option<u32>,
    pub cluster_view_area_bottom: Option<u32>,
    pub cluster_view_area_left: Option<u32>,
    pub cluster_view_area_right: Option<u32>,
    pub cluster_safe_area_top: Option<u32>,
    pub cluster_safe_area_bottom: Option<u32>,
    pub cluster_safe_area_left: Option<u32>,
    pub cluster_safe_area_right: Option<u32>,
    pub disable_audio_output: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            hu_name: None,
            video_width: None,
            video_height: None,
            video_dpi: None,
            video_fps: None,
            pixel_aspect_ratio_e4: None,
            display_width: None,
            display_height: None,
            main_view_area_top: None,
            main_view_area_bottom: None,
            main_view_area_left: None,
            main_view_area_right: None,
            main_safe_area_top: None,
            main_safe_area_bottom: None,
            main_safe_area_left: None,
            main_safe_area_right: None,
            driver_position: 0,
            bt_mac_address: None,
            wifi_bssid: None,
            wifi_ssid: None,
            wifi_password: None,
            fuel_types: Vec::new(),
            ev_connector_types: Vec::new(),
            hevc_supported: false,
            vp9_supported: false,
            av1_supported: false,
            initial_night_mode: false,
            cluster_enabled: false,
            cluster_width: 0,
            cluster_height: 0,
            cluster_tier_width: None,
            cluster_tier_height: None,
            cluster_pixel_aspect_ratio_e4: None,
            cluster_fps: 30,
            cluster_dpi: 140,
            cluster_view_area_top: None,
            cluster_view_area_bottom: None,
            cluster_view_area_left: None,
            cluster_view_area_right: None,
            cluster_safe_area_top: None,
            cluster_safe_area_bottom: None,
            cluster_safe_area_left: None,
            cluster_safe_area_right: None,
            disable_audio_output: false,
        }
    }
}
