//! Radio channel handler (`CH.RADIO` = 11).
//!
//!
//!

mod radio_msg {
    pub const RADIO_MESSAGE_ACTIVE_RADIO_NOTIFICATION: u16 = 32769;
    pub const RADIO_MESSAGE_SELECT_ACTIVE_RADIO_REQUEST: u16 = 32770;
    pub const RADIO_MESSAGE_STEP_CHANNEL_REQUEST: u16 = 32771;
    pub const RADIO_MESSAGE_STEP_CHANNEL_RESPONSE: u16 = 32772;
    pub const RADIO_MESSAGE_SEEK_STATION_REQUEST: u16 = 32773;
    pub const RADIO_MESSAGE_SEEK_STATION_RESPONSE: u16 = 32774;
    pub const RADIO_MESSAGE_SCAN_STATIONS_REQUEST: u16 = 32775;
    pub const RADIO_MESSAGE_SCAN_STATIONS_RESPONSE: u16 = 32776;
    pub const RADIO_MESSAGE_TUNE_TO_STATION_REQUEST: u16 = 32777;
    pub const RADIO_MESSAGE_TUNE_TO_STATION_RESPONSE: u16 = 32778;
    pub const RADIO_MESSAGE_GET_PROGRAM_LIST_REQUEST: u16 = 32779;
    pub const RADIO_MESSAGE_GET_PROGRAM_LIST_RESPONSE: u16 = 32780;
    pub const RADIO_MESSAGE_STATION_PRESETS_NOTIFICATION: u16 = 32781;
    pub const RADIO_MESSAGE_CANCEL_OPERATIONS_REQUEST: u16 = 32782;
    pub const RADIO_MESSAGE_CANCEL_OPERATIONS_RESPONSE: u16 = 32783;
    pub const RADIO_MESSAGE_CONFIGURE_CHANNEL_SPACING_REQUEST: u16 = 32784;
    pub const RADIO_MESSAGE_CONFIGURE_CHANNEL_SPACING_RESPONSE: u16 = 32785;
    pub const RADIO_MESSAGE_RADIO_STATION_INFO_NOTIFICATION: u16 = 32786;
    pub const RADIO_MESSAGE_MUTE_RADIO_REQUEST: u16 = 32787;
    pub const RADIO_MESSAGE_MUTE_RADIO_RESPONSE: u16 = 32788;
    pub const RADIO_MESSAGE_GET_TRAFFIC_UPDATE_REQUEST: u16 = 32789;
    pub const RADIO_MESSAGE_GET_TRAFFIC_UPDATE_RESPONSE: u16 = 32790;
    pub const RADIO_MESSAGE_RADIO_SOURCE_REQUEST: u16 = 32791;
    pub const RADIO_MESSAGE_RADIO_SOURCE_RESPONSE: u16 = 32792;
    pub const RADIO_MESSAGE_STATE_NOTIFICATION: u16 = 32793;
}

pub struct RadioStationInfo {
    pub radio_type: RadioType,
    pub channel: u32,
    pub sub_channel: Option<u32>,
    pub meta_data: Option<RadioStationMetaData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioType {
    AM_RADIO,
    #[default]
    FM_RADIO,
    AM_HD_RADIO,
    FM_HD_RADIO,
    DAB_RADIO,
    XM_RADIO,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ItuRegion {
    #[default]
    RADIO_REGION_NONE,
    RADIO_REGION_ITU_1,
    RADIO_REGION_ITU_2,
    RADIO_REGION_OIRT,
    RADIO_REGION_JAPAN,
    RADIO_REGION_KOREA,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RdsType {
    #[default]
    NO_RDS,
    RDS,
    RBDS,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrafficServiceType {
    #[default]
    NO_TRAFFIC_SERVICE,
    TMC_TRAFFIC_SERVICE,
}

#[derive(Debug, Clone, Default)]
pub struct RadioStationMetaData {
    pub audio_channels: Option<u32>,
    pub signal_quality: Option<u32>,
    pub rds: Option<RdsData>,
}

#[derive(Debug, Clone, Default)]
pub struct RdsData {
    pub alternative_frequencies: Option<Vec<u32>>,
    pub program_id: Option<u32>,
    pub music_speech_switch: Option<u32>,
    pub program_service_name: Option<String>,
    pub program_type: Option<u32>,
    pub program_type_name: Option<String>,
    pub radio_text: Option<String>,
    pub traffic_program_flag: Option<bool>,
    pub traffic_announcement_flag: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct RadioProperties {
    pub radio_id: i32,
    pub radio_type: RadioType,
    pub channel_range: Vec<Range>,
    pub channel_spacings: Vec<i32>,
    pub channel_spacing: i32,
    pub background_tuner: Option<bool>,
    pub region: Option<ItuRegion>,
    pub rds: Option<RdsType>,
    pub af_switch: Option<bool>,
    pub ta: Option<bool>,
    pub traffic_service: Option<TrafficServiceType>,
    pub audio_loopback: Option<bool>,
    pub mute_capability: Option<bool>,
    pub station_presets_access: Option<i32>,
}

#[derive(Debug, Clone, Default)]
pub struct Range {
    pub min: u32,
    pub max: u32,
}

pub struct RadioChannel {
    channel_id: u8,
}

impl RadioChannel {
    pub fn new(channel_id: u8) -> Self {
        Self { channel_id }
    }

    pub fn handle_message(&self, msg_id: u16, payload: &[u8]) {
        println!(
            "[RadioChannel ch={}] received msgId=0x{msg_id:04x} len={}",
            self.channel_id,
            payload.len()
        );
        match msg_id {
            radio_msg::RADIO_MESSAGE_ACTIVE_RADIO_NOTIFICATION => {
                // Handle active radio notification
            }
            radio_msg::RADIO_MESSAGE_SELECT_ACTIVE_RADIO_REQUEST => {
                // Handle select active radio request
            }
            // Add handling for other message types as needed
            _ => {
                eprintln!("Unknown radio message ID: {}", msg_id);
            }
        }
    }

    pub fn handle_setup_request(&self, payload: &[u8]) {
        // Handle setup request for the radio channel
        println!(
            "[RadioChannel ch={}] handling setup request, len={}",
            self.channel_id,
            payload.len()
        );
    }
}
