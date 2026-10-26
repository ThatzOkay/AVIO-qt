//! `DabService` / `DabServiceComponent` equivalents: the per-service and
//! per-service-component records built up from FIC (Fast Information
//! Channel) FIGs. See [`crate::ensemble`] for how these are assembled from
//! [`crate::fic::FigEvent`]s.
//!
//! Reference: `dabservice.h/.cpp`, `dabservicecomponent.h/.cpp`,
//! `dabservicecomponentmscstreamaudio.h`, `dabservicecomponentmscstreamdata.h`,
//! `dabservicecomponentmscpacketdata.h`, `dabuserapplication.h`.
//!
//! This module only carries the FIC-signalled bookkeeping fields (labels,
//! protection/bitrate, programme type, user applications, ...) -- the MSC
//! data-input side (`componentMscDataInput`, packet reassembly, the DAB+
//! decoder) is a later phase (`dabplus.rs`/subchannel demux), not part of
//! this database.

use crate::registered_tables::{self, CharacterSet, DataServiceComponentType, UserApplicationType};

/// A user application signalled for a service component (FIG 0/13), e.g.
/// Dynamic Label or MOT Slideshow. Mirrors `DabUserApplication`.
#[derive(Debug, Clone, PartialEq)]
pub struct UserApplication {
    pub user_app_type: UserApplicationType,
    pub is_ca_applied: bool,
    pub is_xpad_app: bool,
    pub xpad_app_type: u8,
    pub data_groups_used: bool,
    pub data_service_component_type: DataServiceComponentType,
    pub ca_organization: u16,
    pub user_app_data: Vec<u8>,
}

/// A key identifying a service component within an [`crate::ensemble::Ensemble`]'s
/// component tables: stream components are indexed by subchannel id, packet
/// components by their (data) service component id. Mirrors the two
/// separate maps `DabEnsemble` keeps
/// (`m_streamComponentsMap`/`m_packetComponentsMap`) -- kept as two maps
/// here too rather than one, since the id spaces aren't shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentKey {
    Stream(u8),
    Packet(u16),
}

/// The `DabServiceComponent::SERVICECOMPONENTTYPE`-specific fields, plus
/// which concrete component type this is (`DabServiceComponentMscStreamAudio`
/// / `MscStreamData` / `MscPacketData`).
#[derive(Debug, Clone)]
pub enum ServiceComponentKind {
    StreamAudio {
        /// ASCTY, raw code (no registered-value enum exists for it in the
        /// reference -- `dabplus.rs`, a later phase, interprets it).
        audio_service_component_type: u8,
    },
    StreamData {
        /// DSCTy, raw code as signalled in FIG 0/2 (kept as the raw byte
        /// here to match `DabServiceComponentMscStreamData::m_dscTy`,
        /// unlike the packet-mode variant below which the reference
        /// resolves through the registered table).
        data_service_component_type: u8,
    },
    PacketData {
        service_component_id: u16,
        data_service_component_type: DataServiceComponentType,
        /// 0xFFFF while unknown (set by FIG 0/3).
        packet_address: u16,
        ca_organization: u16,
        data_group_transport_used: bool,
    },
}

/// A single service component's FIC-signalled bookkeeping. Mirrors
/// `DabServiceComponent` and its three subclasses.
#[derive(Debug, Clone)]
pub struct ServiceComponent {
    pub is_primary: bool,
    pub is_ca_applied: bool,
    /// 0xFF while unknown; always set for stream components (FIG 0/2), set
    /// for packet components once FIG 0/3 arrives.
    pub subchannel_id: u8,
    /// ScIdS; 0xFF while unknown (set by FIG 0/8).
    pub component_id_within_service: u8,
    /// 0xFFFF while unknown (set by FIG 0/1).
    pub msc_start_address: u16,
    /// 0xFFFF while unknown (set by FIG 0/1).
    pub subchannel_size: u16,
    pub convolutional_coding_rate: String,
    pub protection_level_string: String,
    /// 0xFF while unknown (set by FIG 0/1).
    pub protection_level: u8,
    /// `PROTECTIONTYPE` raw value (0=UEP, 1=EEP A, 2=EEP B); 0xFF while
    /// unknown (set by FIG 0/1).
    pub protection_type: u8,
    /// 0xFF while unknown / not UEP-protected.
    pub uep_table_index: u8,
    pub subchannel_bitrate: u16,
    pub is_fec_scheme_applied: bool,
    pub label_charset: CharacterSet,
    pub label: String,
    pub short_label: String,
    pub user_applications: Vec<UserApplication>,
    pub kind: ServiceComponentKind,
}

impl ServiceComponent {
    pub(crate) fn new(kind: ServiceComponentKind) -> Self {
        Self {
            is_primary: false,
            is_ca_applied: false,
            subchannel_id: 0xFF,
            component_id_within_service: 0xFF,
            msc_start_address: 0xFFFF,
            subchannel_size: 0xFFFF,
            convolutional_coding_rate: String::new(),
            protection_level_string: String::new(),
            protection_level: 0xFF,
            protection_type: 0xFF,
            uep_table_index: 0xFF,
            subchannel_bitrate: 0,
            is_fec_scheme_applied: false,
            label_charset: CharacterSet::EbuLatin,
            label: String::new(),
            short_label: String::new(),
            user_applications: Vec::new(),
            kind,
        }
    }

    /// `DabServiceComponent::SERVICECOMPONENTTYPE` equivalent.
    pub fn is_packet_data(&self) -> bool {
        matches!(self.kind, ServiceComponentKind::PacketData { .. })
    }

    /// Mirrors `DabServiceComponent::setServiceComponentIdWithinService`:
    /// ScIdS 0 marks the primary component of its service.
    pub(crate) fn set_component_id_within_service(&mut self, sc_id_s: u8) {
        self.component_id_within_service = sc_id_s;
        self.is_primary = sc_id_s == 0;
    }

    /// Mirrors `DabServiceComponent::setServiceComponentLabel`/
    /// `setServiceComponentShortLabel`: only the first non-empty label
    /// received is kept.
    pub(crate) fn set_label(&mut self, charset: CharacterSet, label: &str, short_label: &str) {
        self.label_charset = charset;
        if self.label.is_empty() {
            self.label = label.to_string();
        }
        if self.short_label.is_empty() {
            self.short_label = short_label.to_string();
        }
    }

    /// Mirrors `DabServiceComponent::addUserApplication`: de-duplicated by
    /// full equality.
    pub(crate) fn add_user_application(&mut self, app: UserApplication) {
        if !self.user_applications.contains(&app) {
            self.user_applications.push(app);
        }
    }
}

/// A single service's FIC-signalled bookkeeping. Mirrors `DabService`.
#[derive(Debug, Clone)]
pub struct Service {
    pub service_id: u32,
    pub is_programme_service: bool,
    pub ca_id: u8,
    pub is_ca_applied: bool,
    pub num_service_components: u8,
    pub label_charset: CharacterSet,
    pub label: String,
    pub short_label: String,
    /// Keys into the owning [`crate::ensemble::Ensemble`]'s component
    /// tables, in the order components were added -- mirrors
    /// `m_components` (a `vector<shared_ptr<DabServiceComponent>>`), kept
    /// as keys instead of shared references since Rust favours a single
    /// owner (the ensemble's component maps) over shared mutable state.
    pub component_keys: Vec<ComponentKey>,
    pub programme_type_code: u8,
    pub programme_type_is_dynamic: bool,
    pub ensemble_frequency_khz: u32,
}

impl Service {
    pub(crate) fn new(service_id: u32) -> Self {
        Self {
            service_id,
            is_programme_service: false,
            ca_id: 0,
            is_ca_applied: false,
            num_service_components: 0,
            label_charset: CharacterSet::EbuLatin,
            label: String::new(),
            short_label: String::new(),
            component_keys: Vec::new(),
            programme_type_code: 0,
            programme_type_is_dynamic: false,
            ensemble_frequency_khz: 0,
        }
    }

    /// Mirrors `DabService::setCaId`, which (via `m_caId = m_caApplied =
    /// caId`) derives `isCaApplied()` from whether the CA id is nonzero.
    pub(crate) fn set_ca_id(&mut self, ca_id: u8) {
        self.ca_id = ca_id;
        self.is_ca_applied = ca_id != 0;
    }

    /// Mirrors `DabService::setServiceLabel`/`setServiceShortLabel`: only
    /// the first non-empty label received is kept.
    pub(crate) fn set_label(&mut self, charset: CharacterSet, label: &str, short_label: &str) {
        self.label_charset = charset;
        if self.label.is_empty() {
            self.label = label.to_string();
        }
        if self.short_label.is_empty() {
            self.short_label = short_label.to_string();
        }
    }

    /// Mirrors `DabService::setProgrammeTypeCode`: ignores an out-of-range
    /// code (the table only has 32 entries).
    pub(crate) fn set_programme_type_code(&mut self, code: u8) {
        if code < 32 {
            self.programme_type_code = code;
        }
    }

    /// `DabService::getProgrammeTypeFullName`/`get16charName`/`get8CharName`,
    /// looked up live from [`registered_tables::PROGRAMME_TYPE_NAME`]
    /// rather than cached on every `setProgrammeTypeCode` call like the
    /// reference does -- avoids the reference's own quirk where a service
    /// whose PTY is never (re)signalled keeps its hardcoded default
    /// ("No program type"/"None"/"None") instead of the table's actual
    /// code-0 entry ("No programme type"/"None"/"None"), since here there's
    /// no separate cached-vs-table state to drift.
    pub fn programme_type_full_name(&self) -> &'static str {
        registered_tables::PROGRAMME_TYPE_NAME[self.programme_type_code as usize][0]
    }

    pub fn programme_type_16char_name(&self) -> &'static str {
        registered_tables::PROGRAMME_TYPE_NAME[self.programme_type_code as usize][1]
    }

    pub fn programme_type_8char_name(&self) -> &'static str {
        registered_tables::PROGRAMME_TYPE_NAME[self.programme_type_code as usize][2]
    }
}
