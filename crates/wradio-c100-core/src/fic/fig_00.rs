//! FIG type 0 extension parsers (multiplex configuration information).
//!
//! Each extension corresponds to one ETS 300 401 clause, cited in its own
//! doc comment below (mirroring the reference implementation's per-file
//! header comments) -- read that before touching a given parser rather
//! than guessing its meaning from the FIG number alone.

use crate::fic::fig::{Fig00Header, FigCursor};
use crate::registered_tables::{AnnouncementType, DataServiceComponentType, UserApplicationType};

// ---------------------------------------------------------------------
// FIG 0/0 -- ETS 300 401 clause 6.4.1: Ensemble information
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeFlag {
    NoChange,
    NextSubchannelOrganization,
    NextServiceOrganization,
    CompleteNextMci,
}

#[derive(Debug, Clone, Copy)]
pub struct Fig0000 {
    pub ensemble_id: u16,
    pub change_flag: ChangeFlag,
    pub is_alarm_supported: bool,
    pub cif_count_high: u8,
    pub cif_count_low: u8,
    /// Only meaningful when `change_flag != NoChange`.
    pub occurrence_change: u8,
}

pub fn parse_fig_00_00(fig_data: &[u8]) -> Vec<Fig0000> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.has_remaining() {
        let Some(ensemble_id) = cur.u16() else { break };
        let Some(b) = cur.u8() else { break };
        let change_flag = match (b & 0xC0) >> 6 {
            0 => ChangeFlag::NoChange,
            1 => ChangeFlag::NextSubchannelOrganization,
            2 => ChangeFlag::NextServiceOrganization,
            _ => ChangeFlag::CompleteNextMci,
        };
        let is_alarm_supported = (b & 0x20) != 0;
        let cif_count_high = b & 0x1F;
        let Some(cif_count_low) = cur.u8() else { break };
        let occurrence_change = if change_flag != ChangeFlag::NoChange {
            cur.u8().unwrap_or(0)
        } else {
            0
        };

        out.push(Fig0000 {
            ensemble_id,
            change_flag,
            is_alarm_supported,
            cif_count_high,
            cif_count_low,
            occurrence_change,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/1 -- ETS 300 401 clause 6.2.1: Basic sub-channel organization
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionLevel {
    Level1A,
    Level2A,
    Level3A,
    Level4A,
    Level1B,
    Level2B,
    Level3B,
    Level4B,
}

impl ProtectionLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Level1A => "1A",
            Self::Level2A => "2A",
            Self::Level3A => "3A",
            Self::Level4A => "4A",
            Self::Level1B => "1B",
            Self::Level2B => "2B",
            Self::Level3B => "3B",
            Self::Level4B => "4B",
        }
    }

    fn from_index(idx: u8) -> Self {
        match idx & 0x07 {
            0 => Self::Level1A,
            1 => Self::Level2A,
            2 => Self::Level3A,
            3 => Self::Level4A,
            4 => Self::Level1B,
            5 => Self::Level2B,
            6 => Self::Level3B,
            _ => Self::Level4B,
        }
    }
}

/// `[subchannel size (CU), protection level, subchannel size (CU)]` for the
/// 8 short-form UEP protection levels, indexed `[protection_level]`.
const PROTECTIONLEVEL_SUBCHAN_SIZE: [u8; 8] = [12, 8, 6, 4, 27, 21, 18, 15];
const CONVOLUTIONAL_CODING_RATE: [&str; 8] =
    ["1/4", "3/8", "1/2", "3/4", "4/9", "4/7", "4/6", "4/5"];

/// `[subchannel size (CU), protection level, bit rate (kbit/s)]` for each of
/// the 64 short-form UEP table indices.
const DAB_SUBCHANNEL_SHORTFORM_PROTECTION_TABLE: [[u16; 3]; 64] = [
    [16, 5, 32],
    [21, 4, 32],
    [24, 3, 32],
    [29, 2, 32],
    [35, 1, 32],
    [24, 5, 48],
    [29, 4, 48],
    [35, 3, 48],
    [42, 2, 48],
    [52, 1, 48],
    [29, 5, 56],
    [35, 4, 56],
    [42, 3, 56],
    [52, 2, 56],
    [32, 5, 64],
    [42, 4, 64],
    [48, 3, 64],
    [58, 2, 64],
    [70, 1, 64],
    [40, 5, 80],
    [52, 4, 80],
    [58, 3, 80],
    [70, 2, 80],
    [84, 1, 80],
    [48, 5, 96],
    [58, 4, 96],
    [70, 3, 96],
    [84, 2, 96],
    [104, 1, 96],
    [58, 5, 112],
    [70, 4, 112],
    [84, 3, 112],
    [104, 2, 112],
    [64, 5, 128],
    [84, 4, 128],
    [96, 3, 128],
    [116, 2, 128],
    [140, 1, 128],
    [80, 5, 160],
    [104, 4, 160],
    [116, 3, 160],
    [140, 2, 160],
    [168, 1, 160],
    [96, 5, 192],
    [116, 4, 192],
    [140, 3, 192],
    [168, 2, 192],
    [208, 1, 192],
    [116, 5, 224],
    [140, 4, 224],
    [168, 3, 224],
    [208, 2, 224],
    [232, 1, 224],
    [128, 5, 256],
    [168, 4, 256],
    [192, 3, 256],
    [232, 2, 256],
    [280, 1, 256],
    [160, 5, 320],
    [208, 4, 320],
    [280, 2, 320],
    [192, 5, 384],
    [280, 3, 384],
    [416, 1, 384],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionType {
    Uep,
    EepA8n,
    EepB32n,
}

#[derive(Debug, Clone)]
pub struct BasicSubchannelOrganization {
    pub is_programme_service: bool,
    pub sub_channel_id: u8,
    pub start_address: u16,
    pub sub_channel_size: u16,
    pub sub_channel_bitrate: u16,
    pub convolutional_coding_rate: &'static str,
    pub protection_level_type: ProtectionLevel,
    pub protection_level: u8,
    pub protection_type: ProtectionType,
    pub uep_table_index: u8,
}

pub fn parse_fig_00_01(fig_data: &[u8], header: Fig00Header) -> Vec<BasicSubchannelOrganization> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 2 {
        let b0 = cur.peek().unwrap();
        let sub_channel_id = (b0 & 0xFC) >> 2;
        let Some(hi_lo) = cur.u16() else { break };
        let start_address = hi_lo & 0x03FF;

        let Some(b2) = cur.u8() else { break };
        let long_form = (b2 & 0x80) != 0;

        let entry = if !long_form {
            let table_index = b2 & 0x3F;
            let Some(row) = DAB_SUBCHANNEL_SHORTFORM_PROTECTION_TABLE.get(table_index as usize)
            else {
                break;
            };
            let protection_level = row[1] as u8;
            BasicSubchannelOrganization {
                is_programme_service: !header.is_data_service,
                sub_channel_id,
                start_address,
                sub_channel_size: row[0],
                sub_channel_bitrate: row[2],
                convolutional_coding_rate: CONVOLUTIONAL_CODING_RATE[protection_level as usize],
                protection_level_type: ProtectionLevel::from_index(protection_level),
                protection_level,
                protection_type: ProtectionType::Uep,
                uep_table_index: table_index,
            }
        } else {
            let option = (b2 & 0x70) >> 4;
            let protection_level = (b2 & 0x0C) >> 2;
            // Re-derive the 10-bit subchannel size from the byte we already
            // consumed plus one more.
            let sub_channel_size = (((b2 & 0x03) as u16) << 8) | cur.u8().unwrap_or(0) as u16;

            match option {
                0 => BasicSubchannelOrganization {
                    is_programme_service: !header.is_data_service,
                    sub_channel_id,
                    start_address,
                    sub_channel_size,
                    sub_channel_bitrate: (sub_channel_size * 8)
                        / PROTECTIONLEVEL_SUBCHAN_SIZE[protection_level as usize] as u16,
                    convolutional_coding_rate: CONVOLUTIONAL_CODING_RATE[protection_level as usize],
                    protection_level_type: ProtectionLevel::from_index(protection_level),
                    protection_level,
                    protection_type: ProtectionType::EepA8n,
                    uep_table_index: 0xFF,
                },
                1 => {
                    let idx = protection_level + 4;
                    BasicSubchannelOrganization {
                        is_programme_service: !header.is_data_service,
                        sub_channel_id,
                        start_address,
                        sub_channel_size,
                        sub_channel_bitrate: (sub_channel_size * 32)
                            / PROTECTIONLEVEL_SUBCHAN_SIZE[idx as usize] as u16,
                        convolutional_coding_rate: CONVOLUTIONAL_CODING_RATE[idx as usize],
                        protection_level_type: ProtectionLevel::from_index(idx),
                        protection_level,
                        protection_type: ProtectionType::EepB32n,
                        uep_table_index: 0xFF,
                    }
                }
                // RFU option values: not defined by the spec, nothing further to extract.
                _ => break,
            }
        };

        out.push(entry);
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/2 -- ETS 300 401 clause 6.3.1: Basic service and service component definition
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tmid {
    MscStreamAudio,
    MscStreamData,
    Reserved,
    MscPacketModeData,
}

#[derive(Debug, Clone, Copy)]
pub struct ServiceComponentDescription {
    pub transport_mode_id: Tmid,
    /// Only valid for `MscStreamAudio`/`MscStreamData`.
    pub sub_channel_id: u8,
    pub is_primary: bool,
    pub is_ca_applied: bool,
    /// Only valid for `MscStreamAudio`/`MscStreamData`.
    pub service_component_type: u8,
    /// Only valid for `MscPacketModeData`.
    pub service_component_id: u16,
}

#[derive(Debug, Clone)]
pub struct ServiceDescription {
    pub service_id: u32,
    /// Zero if no Conditional Access is used for the service.
    pub ca_id: u8,
    pub num_service_components: u8,
    pub is_programme_service: bool,
    pub service_components: Vec<ServiceComponentDescription>,
}

pub fn parse_fig_00_02(fig_data: &[u8], header: Fig00Header) -> Vec<ServiceDescription> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.has_remaining() {
        let is_programme_service = !header.is_data_service;
        let service_id = if is_programme_service {
            let Some(v) = cur.u16() else { break };
            v as u32
        } else {
            let Some(v) = cur.u32() else { break };
            v
        };

        let Some(b) = cur.u8() else { break };
        let ca_id = (b & 0x70) >> 4;
        let num_service_components = b & 0x0F;

        let mut service_components = Vec::new();
        for _ in 0..num_service_components {
            let Some(b) = cur.peek() else { break };
            let transport_mode_id = match (b & 0xC0) >> 6 {
                0 => Tmid::MscStreamAudio,
                1 => Tmid::MscStreamData,
                2 => Tmid::Reserved,
                _ => Tmid::MscPacketModeData,
            };

            let comp = match transport_mode_id {
                Tmid::MscStreamAudio | Tmid::MscStreamData => {
                    let Some(b0) = cur.u8() else { break };
                    let service_component_type = b0 & 0x3F;
                    let Some(b1) = cur.u8() else { break };
                    ServiceComponentDescription {
                        transport_mode_id,
                        sub_channel_id: (b1 & 0xFC) >> 2,
                        is_primary: (b1 & 0x02) != 0,
                        is_ca_applied: (b1 & 0x01) != 0,
                        service_component_type,
                        service_component_id: 0xFFFF,
                    }
                }
                Tmid::MscPacketModeData => {
                    let Some(b0) = cur.u8() else { break };
                    let Some(b1) = cur.u8() else { break };
                    let service_component_id =
                        ((b0 & 0x3F) as u16) << 6 | ((b1 & 0xFC) as u16) >> 2;
                    ServiceComponentDescription {
                        transport_mode_id,
                        sub_channel_id: 0xFF,
                        is_primary: (b1 & 0x02) != 0,
                        is_ca_applied: (b1 & 0x01) != 0,
                        service_component_type: 0xFF,
                        service_component_id,
                    }
                }
                Tmid::Reserved => {
                    // Obsolete FIDC slot: the reference doesn't advance the
                    // cursor here either, which corrupts the remainder of
                    // this service's component list -- there is nothing
                    // well-defined to parse.
                    break;
                }
            };

            service_components.push(comp);
        }

        out.push(ServiceDescription {
            service_id,
            ca_id,
            num_service_components,
            is_programme_service,
            service_components,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/3 -- ETS 300 401 clause 6.3.2: Service component in packet mode with or without CA
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PacketModeServiceComponentDescription {
    pub is_programme_service: bool,
    pub service_component_id: u16,
    pub ca_organization: Option<u16>,
    pub data_group_transport_used: bool,
    pub data_service_component_type: DataServiceComponentType,
    pub subchannel_id: u8,
    pub packet_address: u16,
}

pub fn parse_fig_00_03(
    fig_data: &[u8],
    header: Fig00Header,
) -> Vec<PacketModeServiceComponentDescription> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 4 {
        let Some(b0) = cur.u8() else { break };
        let Some(b1) = cur.u8() else { break };
        let service_component_id = ((b0 as u16) << 4) | ((b1 as u16 & 0xF0) >> 4);
        let ca_organization_present = (b1 & 0x01) != 0;

        let Some(b2) = cur.u8() else { break };
        let data_group_transport_used = (b2 & 0x80) == 0;
        let data_service_component_type = DataServiceComponentType::from_code(b2 & 0x3F);

        let Some(hi_lo) = cur.u16() else { break };
        let subchannel_id = ((hi_lo >> 8) as u8 & 0xFC) >> 2;
        let packet_address = hi_lo & 0x03FF;

        let ca_organization = if ca_organization_present {
            cur.u16()
        } else {
            None
        };

        out.push(PacketModeServiceComponentDescription {
            is_programme_service: !header.is_data_service,
            service_component_id,
            ca_organization,
            data_group_transport_used,
            data_service_component_type,
            subchannel_id,
            packet_address,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/4 -- ETS 300 401 clause 6.3.3: Service component with CA in stream mode
// ---------------------------------------------------------------------

/// Signaling only -- there is no key material available to actually
/// descramble Conditional-Access content, and this crate doesn't attempt
/// to.
///
/// The reference implementation parses this FIG's fields into local
/// variables and discards them without storing or exposing anything; kept
/// here instead, since a full port should surface what it parses.
#[derive(Debug, Clone, Copy)]
pub struct Fig0004 {
    pub sub_channel_id: u8,
    pub ca_organization: u16,
}

pub fn parse_fig_00_04(fig_data: &[u8]) -> Vec<Fig0004> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 3 {
        let Some(b) = cur.u8() else { break };
        let sub_channel_id = b & 0x3F;
        let Some(ca_organization) = cur.u16() else {
            break;
        };
        out.push(Fig0004 {
            sub_channel_id,
            ca_organization,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/5 -- ETS 300 401 clause 8.1.2: Service component language
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct Fig0005 {
    pub language_code: u8,
    pub is_short_form: bool,
    /// Present when `is_short_form`.
    pub sub_channel_id: Option<u8>,
    /// Present when `!is_short_form`.
    pub service_component_id: Option<u16>,
}

pub fn parse_fig_00_05(fig_data: &[u8]) -> Vec<Fig0005> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.has_remaining() {
        let Some(b) = cur.peek() else { break };
        let is_short_form = (b & 0x80) == 0;

        let (sub_channel_id, service_component_id) = if is_short_form {
            let Some(b) = cur.u8() else { break };
            (Some(b & 0x3F), None)
        } else {
            let Some(hi_lo) = cur.u16() else { break };
            (None, Some(hi_lo & 0x0FFF))
        };

        let Some(language_code) = cur.u8() else { break };

        out.push(Fig0005 {
            language_code,
            is_short_form,
            sub_channel_id,
            service_component_id,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/6 -- ETS 300 401 clause 8.1.15: Service linking information
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceLinkingIdentifierListQualifier {
    DabSid,
    RdsPi,
    Rfu,
    DrmAmssSid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceLinkList {
    pub id_list_qualifier: ServiceLinkingIdentifierListQualifier,
    pub id_list: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct ServiceLinkingInformation {
    pub is_data_service: bool,
    pub is_change_event: bool,
    pub is_continuation: bool,
    pub linkage_active: bool,
    pub is_soft_link: bool,
    pub is_ils: bool,
    pub linkage_set_number: u16,
    pub link_db_key: u16,
    pub key_service_id: u32,
    pub service_links: Vec<ServiceLinkList>,
}

pub fn parse_fig_00_06(fig_data: &[u8], header: Fig00Header) -> Vec<ServiceLinkingInformation> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 2 {
        let Some(b) = cur.peek() else { break };
        let id_list_flag = (b & 0x80) != 0;
        let la_flag = (b & 0x40) != 0;
        let sh_flag = (b & 0x20) != 0;
        let ils_flag = (b & 0x10) != 0;

        let Some(hi_lo) = cur.u16() else { break };
        let linkage_set_number = hi_lo & 0x0FFF;

        let is_db_start = id_list_flag && !header.is_next_configuration;
        let mut sli = ServiceLinkingInformation {
            is_data_service: header.is_data_service,
            is_change_event: !id_list_flag,
            is_continuation: id_list_flag && header.is_next_configuration,
            linkage_active: la_flag,
            is_soft_link: !sh_flag,
            is_ils: ils_flag,
            linkage_set_number,
            link_db_key: ((header.is_other_ensemble as u16) << 15)
                | ((header.is_data_service as u16) << 14)
                | ((sh_flag as u16) << 13)
                | ((ils_flag as u16) << 12)
                | (linkage_set_number & 0x0FFF),
            key_service_id: 0,
            service_links: Vec::new(),
        };

        let mut sll = ServiceLinkList {
            id_list_qualifier: ServiceLinkingIdentifierListQualifier::DabSid,
            id_list: Vec::new(),
        };

        if id_list_flag {
            let Some(b) = cur.u8() else { break };
            sll.id_list_qualifier = match (b & 0x60) >> 5 {
                0 => ServiceLinkingIdentifierListQualifier::DabSid,
                1 => ServiceLinkingIdentifierListQualifier::RdsPi,
                2 => ServiceLinkingIdentifierListQualifier::Rfu,
                _ => ServiceLinkingIdentifierListQualifier::DrmAmssSid,
            };
            let num_ids = b & 0x0F;

            for i in 0..num_ids {
                let id = if !header.is_data_service && !ils_flag {
                    let Some(v) = cur.u16() else { break };
                    v as u32
                } else if !header.is_data_service && ils_flag {
                    let Some(ecc) = cur.u8() else { break };
                    let Some(sid) = cur.u16() else { break };
                    ((ecc as u32) << 16) | sid as u32
                } else {
                    let Some(v) = cur.u32() else { break };
                    v
                };

                if i == 0 && is_db_start {
                    sli.key_service_id = id;
                } else {
                    sll.id_list.push(id);
                }
            }
        }

        sli.service_links.push(sll);
        out.push(sli);
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/7 -- ETS 300 401 clause 6.4.2: Configuration information
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct Fig0007 {
    pub num_services: u8,
    pub count: u16,
}

pub fn parse_fig_00_07(fig_data: &[u8]) -> Vec<Fig0007> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 2 {
        let Some(hi_lo) = cur.u16() else { break };
        out.push(Fig0007 {
            num_services: ((hi_lo >> 10) & 0x3F) as u8,
            count: hi_lo & 0x03FF,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/8 -- ETS 300 401 clause 6.3.5: Service component global definition
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct ServiceComponentGlobalDefinition {
    pub is_programme_service: bool,
    pub service_id: u32,
    pub sc_id_s: u8,
    pub is_short_form: bool,
    /// Present when `is_short_form`.
    pub subchannel_id: u8,
    /// Present when `!is_short_form`.
    pub service_component_id: u16,
}

pub fn parse_fig_00_08(
    fig_data: &[u8],
    header: Fig00Header,
) -> Vec<ServiceComponentGlobalDefinition> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.has_remaining() {
        let is_programme_service = !header.is_data_service;
        let service_id = if is_programme_service {
            let Some(v) = cur.u16() else { break };
            v as u32
        } else {
            let Some(v) = cur.u32() else { break };
            v
        };

        let Some(b) = cur.u8() else { break };
        let ext_flag = (b & 0x80) != 0;
        let sc_id_s = b & 0x0F;

        let Some(b) = cur.peek() else { break };
        let is_short_form = (b & 0x80) == 0;
        let (subchannel_id, service_component_id) = if is_short_form {
            let Some(b) = cur.u8() else { break };
            (b & 0x3F, 0xFFFF)
        } else {
            let Some(hi_lo) = cur.u16() else { break };
            (0xFF, hi_lo & 0x0FFF)
        };

        if ext_flag {
            cur.skip(1);
        }

        out.push(ServiceComponentGlobalDefinition {
            is_programme_service,
            service_id,
            sc_id_s,
            is_short_form,
            subchannel_id,
            service_component_id,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/9 -- ETS 300 401 clause 8.1.3.2: Country, LTO and International table
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Fig0009 {
    /// Local Time Offset, in units of half hours (signed).
    pub local_time_offset_half_hours: i8,
    pub ensemble_ecc: u8,
    pub international_table_id: u8,
}

pub fn parse_fig_00_09(fig_data: &[u8]) -> Vec<Fig0009> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 3 {
        let Some(b) = cur.u8() else { break };
        let extended_field_present = (b & 0x80) != 0;
        let is_negative_offset = (b & 0x20) != 0;
        let mut lto = (b & 0x1F) as i8;
        if is_negative_offset {
            lto = -lto;
        }

        let Some(ensemble_ecc) = cur.u8() else { break };
        let Some(international_table_id) = cur.u8() else {
            break;
        };

        out.push(Fig0009 {
            local_time_offset_half_hours: lto,
            ensemble_ecc,
            international_table_id,
        });

        if extended_field_present {
            // Extended "other ECC services" list -- parsed to keep the
            // cursor aligned for any following repeat of this FIG, but not
            // otherwise surfaced (the reference discards it too).
            while cur.remaining() >= 2 {
                let Some(b0) = cur.u8() else { break };
                let num_services = (b0 & 0xC0) >> 6;
                let Some(_ecc) = cur.u8() else { break };
                for _ in 0..num_services {
                    if cur.u16().is_none() {
                        break;
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/10 -- ETS 300 401 clause 8.1.3.1: Date and time
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct DabTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
    pub milliseconds: i32,
    pub unix_timestamp_seconds: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct Fig0010 {
    pub is_leap_second_pending: bool,
    pub time: DabTime,
}

/// Modified Julian Day to Gregorian year/month/day (the Fliegel & Van
/// Flandern algorithm), ported arithmetic-for-arithmetic from the
/// reference.
fn mjd_to_ymd(mjd: i64) -> (i32, i32, i32) {
    let mut j = mjd + 2400001 + 68569;
    let c = 4 * j / 146097;
    j -= (146097 * c + 3) / 4;
    let y = 4000 * (j + 1) / 1461001;
    j = j - 1461 * y / 4 + 31;
    let m = 80 * j / 2447;
    let day = j - 2447 * m / 80;
    j = m / 11;
    let month = m + 2 - (12 * j);
    let year = 100 * (c - 49) + y + j;
    (year as i32, month as i32, day as i32)
}

/// UTC `struct tm` fields to a Unix timestamp, ignoring any local time
/// zone (a `timegm`-equivalent), ported from the reference's portable
/// `_mkgmtime` fallback.
fn mkgmtime(year: i32, month0: i32, day: i32, hour: i32, minute: i32, second: i32) -> i64 {
    const MONTH_DAY: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

    let mut month = (month0 as i64) % 12;
    let mut year = year as i64 + (month0 as i64) / 12;
    if month < 0 {
        month += 12;
        year -= 1;
    }

    let year_for_leap = if month > 1 { year + 1 } else { year };

    second as i64
        + 60 * (minute as i64
            + 60 * (hour as i64
                + 24 * (MONTH_DAY[month as usize] + day as i64 - 1
                    + 365 * (year - 70)
                    + (year_for_leap - 69) / 4
                    - (year_for_leap - 1) / 100
                    + (year_for_leap + 299) / 400)))
}

pub fn parse_fig_00_10(fig_data: &[u8]) -> Vec<Fig0010> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 4 {
        let Some(b0) = cur.u8() else { break };
        let Some(b1) = cur.u8() else { break };
        let Some(b2) = cur.peek() else { break };
        let mjd = (((b0 & 0x7F) as i64) << 10) | ((b1 as i64) << 2) | (((b2 & 0xC0) as i64) >> 6);

        let Some(b2) = cur.u8() else { break };
        let is_leap_second_pending = (b2 & 0x20) != 0;
        let utc_flag = (b2 & 0x08) != 0;
        let hour_hi = (b2 & 0x07) as i32;

        // `hour` spans the low 3 bits of this byte and the top 2 bits of
        // the next one; `minute` is the low 6 bits of that same next byte
        // (there is no separate byte for it).
        let Some(b3) = cur.u8() else { break };
        let hour = (hour_hi << 2) | ((b3 as i32 & 0xC0) >> 6);
        let minute = (b3 & 0x3F) as i32;

        let (year, month, day) = mjd_to_ymd(mjd);

        let (second, milliseconds) = if utc_flag {
            let Some(b) = cur.peek() else { break };
            let second = ((b & 0xFC) >> 2) as i32;
            let Some(hi_lo) = cur.u16() else { break };
            let milliseconds = (hi_lo & 0x03FF) as i32;
            (second, milliseconds)
        } else {
            (0, 0)
        };

        let unix_timestamp_seconds = mkgmtime(year, month - 1, day, hour, minute, second);

        out.push(Fig0010 {
            is_leap_second_pending,
            time: DabTime {
                year,
                month,
                day,
                hour,
                minute,
                second,
                milliseconds,
                unix_timestamp_seconds,
            },
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/13 -- ETS 300 401 clause 6.3.6: User application information
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct UserApplication {
    pub user_app_type: UserApplicationType,
    pub ca_applied: bool,
    pub is_xpad_data: bool,
    pub xpad_app_type: u8,
    pub data_groups_used: bool,
    pub data_service_component_type: DataServiceComponentType,
    pub ca_organization: u16,
    pub user_app_data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct UserApplicationInformation {
    pub service_id: u32,
    pub is_programme_service: bool,
    pub sc_id_s: u8,
    pub num_user_apps: u8,
    pub user_applications: Vec<UserApplication>,
}

pub fn parse_fig_00_13(fig_data: &[u8], header: Fig00Header) -> Vec<UserApplicationInformation> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.has_remaining() {
        let is_programme_service = !header.is_data_service;
        let service_id = if is_programme_service {
            let Some(v) = cur.u16() else { break };
            v as u32
        } else {
            let Some(v) = cur.u32() else { break };
            v
        };

        let Some(b) = cur.u8() else { break };
        let sc_id_s = (b & 0xF0) >> 4;
        let num_user_apps = b & 0x0F;

        let mut user_applications = Vec::new();
        for _ in 0..num_user_apps {
            let Some(b0) = cur.u8() else { break };
            let Some(b1) = cur.peek() else { break };
            let user_app_type =
                UserApplicationType::from_code(((b0 as u16) << 3) | ((b1 as u16 & 0xE0) >> 5));
            let Some(b1) = cur.u8() else { break };
            let mut u_app_data_length = b1 & 0x1F;
            let is_xpad_data = !header.is_data_service;

            let mut ca_applied = false;
            let mut xpad_app_type = 0u8;
            let mut data_groups_used = false;
            let mut data_service_component_type = DataServiceComponentType::UnspecifiedData;
            let mut ca_organization = 0u16;

            if is_xpad_data && u_app_data_length >= 2 {
                let Some(b) = cur.u8() else { break };
                ca_applied = (b & 0x80) != 0;
                let caorg_flag = (b & 0x40) != 0;
                xpad_app_type = b & 0x1F;

                let Some(b) = cur.u8() else { break };
                data_groups_used = (b & 0x80) == 0;
                data_service_component_type = DataServiceComponentType::from_code(b & 0x3F);

                u_app_data_length -= 2;
                if caorg_flag {
                    let Some(v) = cur.u16() else { break };
                    ca_organization = v;
                    u_app_data_length = u_app_data_length.saturating_sub(2);
                }
            }

            let mut user_app_data = Vec::with_capacity(u_app_data_length as usize);
            for _ in 0..u_app_data_length {
                let Some(b) = cur.u8() else { break };
                user_app_data.push(b);
            }

            user_applications.push(UserApplication {
                user_app_type,
                ca_applied,
                is_xpad_data,
                xpad_app_type,
                data_groups_used,
                data_service_component_type,
                ca_organization,
                user_app_data,
            });
        }

        out.push(UserApplicationInformation {
            service_id,
            is_programme_service,
            sc_id_s,
            num_user_apps,
            user_applications,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/14 -- ETS 300 401 clause 6.2.2: FEC sub-channel organization
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FecScheme {
    NotApplied,
    Applied,
    Reserved0,
    Reserved1,
}

#[derive(Debug, Clone, Copy)]
pub struct FecSchemeDescription {
    pub sub_channel_id: u8,
    pub fec_scheme: FecScheme,
}

pub fn parse_fig_00_14(fig_data: &[u8]) -> Vec<FecSchemeDescription> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.has_remaining() {
        let Some(b) = cur.u8() else { break };
        let fec_scheme = match b & 0x03 {
            0 => FecScheme::NotApplied,
            1 => FecScheme::Applied,
            2 => FecScheme::Reserved0,
            _ => FecScheme::Reserved1,
        };
        out.push(FecSchemeDescription {
            sub_channel_id: (b & 0xFC) >> 2,
            fec_scheme,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/17 -- ETS 300 401 clause 8.1.5: Programme Type
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct ProgrammeTypeInformation {
    pub service_id: u16,
    pub is_dynamic: bool,
    pub int_pty_code: u8,
}

pub fn parse_fig_00_17(fig_data: &[u8]) -> Vec<ProgrammeTypeInformation> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 3 {
        let Some(service_id) = cur.u16() else { break };
        let Some(b) = cur.peek() else { break };
        let is_dynamic = (b & 0x80) != 0;
        cur.skip(1);
        let Some(int_pty_code) = cur.u8() else { break };

        out.push(ProgrammeTypeInformation {
            service_id,
            is_dynamic,
            int_pty_code: int_pty_code & 0x1F,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/18 -- ETS 300 401 clause 8.1.6.1: Announcement support
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AnnouncementSupport {
    pub service_id: u16,
    pub supported_announcements: Vec<AnnouncementType>,
    pub cluster_ids: Vec<u8>,
}

pub fn parse_fig_00_18(fig_data: &[u8]) -> Vec<AnnouncementSupport> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 3 {
        let Some(service_id) = cur.u16() else { break };
        let Some(asu_flags) = cur.u16() else { break };

        let mut supported_announcements = Vec::new();
        for bit in 0..16u8 {
            if (asu_flags >> bit) & 1 != 0 {
                supported_announcements.push(AnnouncementType::from_bit(bit));
            }
        }

        let Some(b) = cur.u8() else { break };
        let num_clusters = b & 0x07;
        let mut cluster_ids = Vec::with_capacity(num_clusters as usize);
        for _ in 0..num_clusters {
            let Some(id) = cur.u8() else { break };
            cluster_ids.push(id);
        }

        out.push(AnnouncementSupport {
            service_id,
            supported_announcements,
            cluster_ids,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/19 -- ETS 300 401 clause 8.1.6.2: Announcement switching
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AnnouncementSwitching {
    pub cluster_id: u8,
    pub announcements_switched: Vec<AnnouncementType>,
    pub is_newly_introduced: bool,
    pub subchannel_id: u8,
}

pub fn parse_fig_00_19(fig_data: &[u8]) -> Vec<AnnouncementSwitching> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 4 {
        let Some(cluster_id) = cur.u8() else { break };
        let Some(asw_flags) = cur.u16() else { break };

        let mut announcements_switched = Vec::new();
        for bit in 0..16u8 {
            if (asw_flags >> bit) & 1 != 0 {
                announcements_switched.push(AnnouncementType::from_bit(bit));
            }
        }

        let Some(b) = cur.u8() else { break };

        out.push(AnnouncementSwitching {
            cluster_id,
            announcements_switched,
            is_newly_introduced: (b & 0x80) != 0,
            subchannel_id: b & 0x3F,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/20 -- ETS 300 401 clause 8.1.4: Service Component Information
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SciChangeType {
    RemainsWithNewSid,
    AddedToEnsemble,
    RemovedFromEnsemble,
    RemovedFromAllEnsembles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SciPartTime {
    OnOffAirContinuously,
    OnOffAirCycle,
}

#[derive(Debug, Clone, Copy)]
pub struct ScheduledServiceChange {
    pub is_ca_applied: bool,
    pub is_advance_notice: bool,
    pub service_component_type: u8,
    pub date: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Present when the target service/ensemble id fields were signalled.
    pub target_service_id: Option<u32>,
    pub target_ensemble_id: Option<u16>,
}

#[derive(Debug, Clone, Copy)]
pub struct Fig0020 {
    pub service_id: u32,
    pub sc_id_s: u8,
    pub change_type: SciChangeType,
    pub part_time: SciPartTime,
    pub scheduled_change: Option<ScheduledServiceChange>,
}

pub fn parse_fig_00_20(fig_data: &[u8], header: Fig00Header) -> Vec<Fig0020> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.has_remaining() {
        let service_id = if !header.is_data_service {
            let Some(v) = cur.u16() else { break };
            v as u32
        } else {
            let Some(v) = cur.u32() else { break };
            v
        };

        let Some(b) = cur.u8() else { break };
        let sc_id_s = (b & 0xF0) >> 4;
        let change_type = match (b & 0x0C) >> 2 {
            0 => SciChangeType::RemainsWithNewSid,
            1 => SciChangeType::AddedToEnsemble,
            2 => SciChangeType::RemovedFromEnsemble,
            _ => SciChangeType::RemovedFromAllEnsembles,
        };
        let part_time = if (b & 0x02) != 0 {
            SciPartTime::OnOffAirCycle
        } else {
            SciPartTime::OnOffAirContinuously
        };
        let sc_flag = (b & 0x01) != 0;

        let scheduled_change = if sc_flag {
            let Some(b) = cur.u8() else { break };
            let is_ca_applied = (b & 0x80) != 0;
            let is_advance_notice = (b & 0x40) != 0;
            let service_component_type = b & 0x3F;

            let Some(b0) = cur.u8() else { break };
            let date = (b0 & 0xF8) >> 3;
            let Some(b1) = cur.u8() else { break };
            let hour = ((b0 & 0x07) << 2) | ((b1 & 0xC0) >> 6);
            let minute = b1 & 0x3F;
            let Some(b2) = cur.peek() else { break };
            let second = (b2 & 0xFC) >> 2;
            let sid_flag = (b2 & 0x02) != 0;
            let Some(b2) = cur.u8() else { break };
            let eid_flag = (b2 & 0x01) != 0;

            let mut target_service_id = None;
            let mut target_ensemble_id = None;
            if sid_flag {
                target_service_id = Some(if !header.is_data_service {
                    let Some(v) = cur.u16() else { break };
                    v as u32
                } else {
                    let Some(v) = cur.u32() else { break };
                    v
                });
                if eid_flag {
                    target_ensemble_id = cur.u16();
                }
            }

            Some(ScheduledServiceChange {
                is_ca_applied,
                is_advance_notice,
                service_component_type,
                date,
                hour,
                minute,
                second,
                target_service_id,
                target_ensemble_id,
            })
        } else {
            None
        };

        out.push(Fig0020 {
            service_id,
            sc_id_s,
            change_type,
            part_time,
            scheduled_change,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/21 -- ETS 300 401 clause 8.1.8: Frequency Information
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyInformationType {
    Unknown,
    DabEnsemble,
    Drm,
    FmRds,
    Amss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DabEnsembleAdjacent {
    Unknown,
    AdjacentModeNotSignalled,
    AdjacentModeOne,
    NotAdjacentModeNotSignalled,
    NotAdjacentModeOne,
}

#[derive(Debug, Clone)]
pub struct FrequencyInformation {
    pub id: u16,
    pub frequencies_khz: Vec<u32>,
    pub frequency_information_type: FrequencyInformationType,
    pub adjacent: DabEnsembleAdjacent,
    pub continuous_output: bool,
    pub is_other_ensemble: bool,
    pub is_change_event: bool,
}

pub fn parse_fig_00_21(fig_data: &[u8], header: Fig00Header) -> Vec<FrequencyInformation> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 2 {
        let Some(_rfa_and_len_hi) = cur.u8() else {
            break;
        };
        let Some(b) = cur.u8() else { break };
        let len_fi_list = (b & 0x1F) as usize;

        let Some(fi_list_data) = cur.take(len_fi_list) else {
            break;
        };
        let mut fi_cur = FigCursor::new(fi_list_data);

        while fi_cur.remaining() >= 3 {
            let Some(id_field) = fi_cur.u16() else { break };
            let Some(b) = fi_cur.u8() else { break };
            let range_modulation = (b & 0xF0) >> 4;
            let cont_flag = (b & 0x08) != 0;
            let freq_list_entry_len = (b & 0x07) as usize;

            let Some(freq_list_data) = fi_cur.take(freq_list_entry_len) else {
                break;
            };
            let mut freq_cur = FigCursor::new(freq_list_data);

            let mut info = FrequencyInformation {
                id: id_field,
                frequencies_khz: Vec::new(),
                frequency_information_type: FrequencyInformationType::Unknown,
                adjacent: DabEnsembleAdjacent::Unknown,
                continuous_output: cont_flag,
                is_other_ensemble: header.is_other_ensemble,
                is_change_event: freq_list_entry_len == 0,
            };

            while freq_cur.has_remaining() {
                match range_modulation {
                    // R&M = 0000: DAB Ensemble
                    0 => {
                        let Some(b) = freq_cur.peek() else { break };
                        let control_field = (b & 0xF8) >> 3;
                        // The reference's switch on this field is missing
                        // `break`s, so every value falls through to the
                        // last case and always resolves to
                        // NotAdjacentModeOne regardless of the actual
                        // control field -- fixed here to select per-value.
                        info.adjacent = match control_field {
                            0 => DabEnsembleAdjacent::AdjacentModeNotSignalled,
                            1 => DabEnsembleAdjacent::NotAdjacentModeNotSignalled,
                            2 => DabEnsembleAdjacent::AdjacentModeOne,
                            3 => DabEnsembleAdjacent::NotAdjacentModeOne,
                            _ => DabEnsembleAdjacent::Unknown,
                        };

                        let Some(freq) = freq_cur.u24() else { break };
                        let freq = freq & 0x0007_FFFF;
                        info.frequency_information_type = FrequencyInformationType::DabEnsemble;
                        info.frequencies_khz.push(freq * 16);
                    }
                    // R&M = 1000: FM with RDS
                    8 => {
                        let Some(freq) = freq_cur.u8() else { break };
                        info.frequency_information_type = FrequencyInformationType::FmRds;
                        info.frequencies_khz.push(freq as u32 * 100 + 87_500);
                    }
                    // R&M = 0110: DRM, R&M = 1110: AMSS
                    6 | 14 => {
                        let Some(id2) = freq_cur.u8() else { break };
                        let Some(b) = freq_cur.peek() else { break };
                        let multiplier = (b & 0x80) != 0;
                        let Some(hi_lo) = freq_cur.u16() else { break };
                        let freq = hi_lo & 0x7FFF;
                        info.frequencies_khz.push(if multiplier {
                            freq as u32 * 10
                        } else {
                            freq as u32
                        });
                        info.frequency_information_type = if range_modulation == 6 {
                            FrequencyInformationType::Drm
                        } else {
                            FrequencyInformationType::Amss
                        };
                        info.id = ((id2 as u16) << 8) | id_field;
                    }
                    _ => break,
                }
            }

            info.frequencies_khz.sort_unstable();
            // The reference pushes a new (cumulatively-growing) entry to
            // its output list on every iteration of this inner loop
            // instead of once per id/range-modulation block, producing
            // duplicate entries with progressively longer frequency lists.
            // Pushed once here instead, after collecting every frequency
            // for this block.
            out.push(info);
        }
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/24 -- ETS 300 401 clause 8.1.10: OE Services
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct OtherEnsembleServiceInformation {
    pub service_id: u32,
    pub ca_id: u8,
    pub ensemble_ids: Vec<u16>,
    pub is_other_ensemble: bool,
    pub is_cei: bool,
}

pub fn parse_fig_00_24(
    fig_data: &[u8],
    header: Fig00Header,
) -> Vec<OtherEnsembleServiceInformation> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.has_remaining() {
        let service_id = if !header.is_data_service {
            let Some(v) = cur.u16() else { break };
            v as u32
        } else {
            let Some(v) = cur.u32() else { break };
            v
        };

        let Some(b) = cur.u8() else { break };
        let ca_id = (b & 0x70) >> 4;
        let num_eids = b & 0x0F;

        let mut ensemble_ids = Vec::with_capacity(num_eids as usize);
        for _ in 0..num_eids {
            let Some(eid) = cur.u16() else { break };
            ensemble_ids.push(eid);
        }
        ensemble_ids.sort_unstable();

        out.push(OtherEnsembleServiceInformation {
            service_id,
            ca_id,
            is_cei: num_eids == 0,
            ensemble_ids,
            is_other_ensemble: header.is_other_ensemble,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/25 -- ETS 300 401 clause 8.1.6.3: OE Announcement support
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct OeAnnouncementSupportTarget {
    pub oe_cluster_id: u8,
    pub ensemble_id: u16,
}

#[derive(Debug, Clone)]
pub struct OeAnnouncementSupport {
    pub service_id: u16,
    pub supported_announcements: Vec<AnnouncementType>,
    pub targets: Vec<OeAnnouncementSupportTarget>,
}

pub fn parse_fig_00_25(fig_data: &[u8]) -> Vec<OeAnnouncementSupport> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 5 {
        let Some(service_id) = cur.u16() else { break };
        let Some(asu_flags) = cur.u16() else { break };
        let mut supported_announcements = Vec::new();
        for bit in 0..16u8 {
            if (asu_flags >> bit) & 1 != 0 {
                supported_announcements.push(AnnouncementType::from_bit(bit));
            }
        }

        let Some(b) = cur.u8() else { break };
        let num_targets = b & 0x03;
        let mut targets = Vec::with_capacity(num_targets as usize);
        for _ in 0..num_targets {
            let Some(oe_cluster_id) = cur.u8() else { break };
            let Some(ensemble_id) = cur.u16() else { break };
            targets.push(OeAnnouncementSupportTarget {
                oe_cluster_id,
                ensemble_id,
            });
        }

        out.push(OeAnnouncementSupport {
            service_id,
            supported_announcements,
            targets,
        });
    }
    out
}

// ---------------------------------------------------------------------
// FIG 0/26 -- ETS 300 401 clause 8.1.6.4: OE Announcement switching
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct OeAnnouncementSwitching {
    pub oe_cluster_id: u8,
    pub announcements_switched: Vec<AnnouncementType>,
    pub is_new: bool,
    pub sc_id_s: u8,
    pub target_service_id: u16,
}

pub fn parse_fig_00_26(fig_data: &[u8]) -> Vec<OeAnnouncementSwitching> {
    let mut out = Vec::new();
    let mut cur = FigCursor::new(&fig_data[1..]);
    while cur.remaining() >= 6 {
        let Some(oe_cluster_id) = cur.u8() else { break };
        let Some(oe_asw_flags) = cur.u16() else { break };
        let mut announcements_switched = Vec::new();
        for bit in 0..16u8 {
            if (oe_asw_flags >> bit) & 1 != 0 {
                announcements_switched.push(AnnouncementType::from_bit(bit));
            }
        }

        let Some(b) = cur.u8() else { break };
        let is_new = (b & 0x80) != 0;
        let sc_id_s = b & 0x0F;
        let Some(target_service_id) = cur.u16() else {
            break;
        };

        out.push(OeAnnouncementSwitching {
            oe_cluster_id,
            announcements_switched,
            is_new,
            sc_id_s,
            target_service_id,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fig_00_00_ensemble_information() {
        // header byte (unused by this parser) + ensemble_id 0xC1F1 +
        // change_flag=NoChange, alarm supported, cifCntHigh=5 + cifCntLow=42.
        let fig_data = [0x00, 0xC1, 0xF1, 0x25, 42];
        let out = parse_fig_00_00(&fig_data);
        assert_eq!(out.len(), 1);
        let fig = out[0];
        assert_eq!(fig.ensemble_id, 0xC1F1);
        assert_eq!(fig.change_flag, ChangeFlag::NoChange);
        assert!(fig.is_alarm_supported);
        assert_eq!(fig.cif_count_high, 5);
        assert_eq!(fig.cif_count_low, 42);
        assert_eq!(fig.occurrence_change, 0);
    }

    #[test]
    fn parses_fig_00_02_one_programme_service_one_audio_component() {
        // header byte (P/D=0 -> programme service) + SId 0x1001 +
        // (caId=0, numServiceComponents=1) + one MSC-stream-audio component
        // (TMID=00, service_component_type=0, subChanId=5, primary, no CA).
        let fig_data = [0x00, 0x10, 0x01, 0x01, 0x00, 0x16];
        let header = Fig00Header::parse(fig_data[0]);
        let out = parse_fig_00_02(&fig_data, header);

        assert_eq!(out.len(), 1);
        let desc = &out[0];
        assert_eq!(desc.service_id, 0x1001);
        assert_eq!(desc.ca_id, 0);
        assert_eq!(desc.num_service_components, 1);
        assert!(desc.is_programme_service);
        assert_eq!(desc.service_components.len(), 1);

        let comp = desc.service_components[0];
        assert_eq!(comp.transport_mode_id, Tmid::MscStreamAudio);
        assert_eq!(comp.sub_channel_id, 5);
        assert!(comp.is_primary);
        assert!(!comp.is_ca_applied);
        assert_eq!(comp.service_component_type, 0);
    }

    #[test]
    fn parses_fig_00_17_programme_type() {
        // service_id 0x1234, dynamic flag set, intPtyCode 10 (Pop Music).
        let fig_data = [0x00, 0x12, 0x34, 0x80, 0x0A];
        let out = parse_fig_00_17(&fig_data);
        assert_eq!(out.len(), 1);
        let fig = out[0];
        assert_eq!(fig.service_id, 0x1234);
        assert!(fig.is_dynamic);
        assert_eq!(fig.int_pty_code, 10);
    }
}
