//! `DabEnsemble` equivalent: the service/component database assembled from
//! parsed FIC events.
//!
//! Reference: `dabensemble.h/.cpp`. Unlike the reference, which drove this
//! incrementally off `FicParser`'s per-FIG callbacks (registered/torn down
//! in a specific order tracked by a pile of `m_fig0Xdone` flags -- see
//! `registerCbs`/`fig_00_done_cb`/`fig_01_done_cb`/`checkServiceSanity` --
//! purely so it knew *when* to start listening for FIGs that only make
//! sense once earlier ones were collected), [`Ensemble::apply`] here just
//! processes every [`crate::fic::FigEvent`] handed to it unconditionally,
//! every time. That callback-ordering dance doesn't buy anything once
//! there's no async callback-registration lifecycle to manage: FIC data is
//! periodic broadcast, the same FIGs keep reappearing every FIC read, and a
//! FIG that references not-yet-seen state (e.g. FIG 0/8 arriving before the
//! service it updates exists) is naturally a no-op lookup miss that just
//! resolves itself on a later call once that state exists -- exactly the
//! steady state the reference's flag dance was built to reach anyway.
//! [`Ensemble::is_collect_finished`] reproduces the reference's actual
//! sanity predicate (`checkServiceSanity`) as a plain on-demand check
//! instead of a cached flag.

use std::collections::HashMap;

use crate::fic::{fig_00, fig_01, FigEvent};
use crate::registered_tables::CharacterSet;
use crate::service::{
    ComponentKey, Service, ServiceComponent, ServiceComponentKind, UserApplication,
};

/// The service/component database for one DAB ensemble, built up from
/// parsed FIC events. Mirrors `DabEnsemble`.
#[derive(Debug, Clone)]
pub struct Ensemble {
    // FIG 0/0 -- ensemble information
    ensemble_id: u16,
    cif_count_high: u8,
    cif_count_low: u8,
    cif_count_high_next: u8,
    cif_count_low_next: u8,
    is_alarm_announcement_supported: bool,

    // FIG 0/9 -- country/LTO/international table
    ensemble_ecc: u8,

    // FIG 1/0 -- ensemble label
    label_charset: CharacterSet,
    label: String,
    short_label: String,

    // FIG 0/10 -- date & time. Latest value only (no
    // `registerDateTimeCallback`/`CallbackDispatcher` here -- see the
    // Async/runtime section of the port task: output channels replace the
    // callback-dispatcher pattern one level up, in `tuner.rs`; this struct
    // just keeps the latest snapshot, like a `watch` channel's value).
    date_time: Option<fig_00::Fig0010>,

    services: HashMap<u32, Service>,
    stream_components: HashMap<u8, ServiceComponent>,
    packet_components: HashMap<u16, ServiceComponent>,

    // Announcements (FIG 0/18 + 0/19). The reference's own
    // `fig00_18_input` body is entirely commented out (so
    // `m_announcementSupports` is declared but never actually populated
    // there), matching this crate's earlier Phase 2 pass -- but Phase 6's
    // `announcements.rs` genuinely needs it (to know which announcement
    // types/clusters a service actually signals support for before
    // switching to one), so it's stored here now, keyed by service id.
    announcement_supports: HashMap<u16, fig_00::AnnouncementSupport>,
    active_announcements: HashMap<u8, fig_00::AnnouncementSwitching>,

    /// Set externally by the tuning layer (mirrors `RaonTunerInput` writing
    /// its subclass-protected `m_ensembleFrequency` directly) -- there is
    /// no FIG that signals a service's own frequency here, it's just
    /// stamped onto every service as it's discovered.
    ensemble_frequency_khz: u32,
}

impl Default for Ensemble {
    fn default() -> Self {
        Self::new()
    }
}

impl Ensemble {
    pub fn new() -> Self {
        Self {
            ensemble_id: 0xFFFF,
            cif_count_high: 0,
            cif_count_low: 0,
            cif_count_high_next: 0,
            cif_count_low_next: 0,
            is_alarm_announcement_supported: false,
            ensemble_ecc: 0xFF,
            label_charset: CharacterSet::EbuLatin,
            label: String::new(),
            short_label: String::new(),
            date_time: None,
            services: HashMap::new(),
            stream_components: HashMap::new(),
            packet_components: HashMap::new(),
            announcement_supports: HashMap::new(),
            active_announcements: HashMap::new(),
            ensemble_frequency_khz: 0,
        }
    }

    /// Mirrors `DabEnsemble::reset`: clears all collected FIC state back to
    /// defaults (e.g. before retuning to a different ensemble), but -- like
    /// the reference -- leaves the tuned frequency alone.
    pub fn reset(&mut self) {
        let frequency_khz = self.ensemble_frequency_khz;
        *self = Self::new();
        self.ensemble_frequency_khz = frequency_khz;
    }

    /// Set by the tuning layer once a frequency is locked; stamped onto
    /// every service discovered from then on. See the field's own doc
    /// comment.
    pub fn set_frequency_khz(&mut self, frequency_khz: u32) {
        self.ensemble_frequency_khz = frequency_khz;
    }

    pub fn frequency_khz(&self) -> u32 {
        self.ensemble_frequency_khz
    }

    // ---- getters (DabEnsemble's public virtual getters) ----

    pub fn ensemble_id(&self) -> u16 {
        self.ensemble_id
    }

    pub fn current_cif_count_high(&self) -> u8 {
        self.cif_count_high
    }

    pub fn current_cif_count_low(&self) -> u8 {
        self.cif_count_low
    }

    pub fn is_alarm_announcement_supported(&self) -> bool {
        self.is_alarm_announcement_supported
    }

    pub fn ensemble_label_charset(&self) -> CharacterSet {
        self.label_charset
    }

    pub fn ensemble_label(&self) -> &str {
        &self.label
    }

    pub fn ensemble_short_label(&self) -> &str {
        &self.short_label
    }

    pub fn ensemble_ecc(&self) -> u8 {
        self.ensemble_ecc
    }

    pub fn date_time(&self) -> Option<&fig_00::Fig0010> {
        self.date_time.as_ref()
    }

    pub fn services(&self) -> impl Iterator<Item = &Service> {
        self.services.values()
    }

    pub fn service(&self, service_id: u32) -> Option<&Service> {
        self.services.get(&service_id)
    }

    pub fn component(&self, key: ComponentKey) -> Option<&ServiceComponent> {
        match key {
            ComponentKey::Stream(id) => self.stream_components.get(&id),
            ComponentKey::Packet(id) => self.packet_components.get(&id),
        }
    }

    /// Whether `service` has a DAB+ (ASCTY 0x3F) audio stream component
    /// with a resolved subchannel/bitrate -- i.e. whether it's actually
    /// playable through [`crate::dabplus::DabPlusDecoder`]. Classic (MPEG
    /// Layer II, `ascty == 0x00`) audio and data-only services (e.g.
    /// TPEG) never are, matching `dabplus.rs`'s explicit scope. Factored
    /// out here (rather than left duplicated across every caller that
    /// needs it -- `device.rs`'s `service_info`, `scan_and_listen.rs`'s
    /// station upserts, and its own service-selection logic all needed
    /// this exact same check) so persisted station records and live
    /// `ServiceInfo`/`Ensemble` queries can never quietly drift apart on
    /// what "playable" means.
    pub fn service_has_dab_plus_audio(&self, service: &Service) -> bool {
        service.component_keys.iter().any(|&key| {
            self.component(key).is_some_and(|component| {
                matches!(
                    component.kind,
                    ServiceComponentKind::StreamAudio {
                        audio_service_component_type
                    } if audio_service_component_type == crate::dabplus::ASCTY_DAB_PLUS
                ) && component.subchannel_id != 0xFF
                    && component.subchannel_bitrate > 0
            })
        })
    }

    pub fn active_announcements(&self) -> impl Iterator<Item = &fig_00::AnnouncementSwitching> {
        self.active_announcements.values()
    }

    /// The announcement types/clusters `service_id` has signalled support
    /// for (FIG 0/18), if any.
    pub fn announcement_support(&self, service_id: u16) -> Option<&fig_00::AnnouncementSupport> {
        self.announcement_supports.get(&service_id)
    }

    /// Mirrors `DabEnsemble::checkServiceSanity`'s pass/fail predicate: true
    /// once every known service's components have resolved MSC placement
    /// (subchannel id, MSC start address, subchannel size) and component
    /// count, and -- for packet-mode components -- a resolved protection
    /// level/type, and -- for audio stream components -- a non-empty
    /// service label and a known ensemble ECC.
    ///
    /// One deliberate behavioural difference from a literal reading of
    /// `checkServiceSanity`: that function returns `true` for an empty
    /// `m_servicesMap` (the loop over it simply never runs), which was
    /// harmless there because it was only ever invoked after the
    /// `m_fig0Xdone` callback-gating already guaranteed FIG 0/2 had
    /// populated at least one service. Without that gating (see this
    /// module's own doc comment), a caller could reasonably ask this before
    /// any FIC has arrived at all, where "finished" would be wrong -- so
    /// this returns `false` on an empty database instead.
    pub fn is_collect_finished(&self) -> bool {
        if self.services.is_empty() {
            return false;
        }

        for service in self.services.values() {
            if service.component_keys.len() != service.num_service_components as usize {
                return false;
            }

            for &key in &service.component_keys {
                let Some(component) = self.component(key) else {
                    return false;
                };

                if component.subchannel_id == 0xFF
                    || component.msc_start_address == 0xFFFF
                    || component.subchannel_size == 0xFFFF
                {
                    return false;
                }

                match component.kind {
                    ServiceComponentKind::PacketData { .. } => {
                        if component.protection_level == 0xFF || component.protection_type == 0xFF {
                            return false;
                        }
                    }
                    ServiceComponentKind::StreamAudio { .. } => {
                        if service.label.is_empty() || self.ensemble_ecc == 0xFF {
                            return false;
                        }
                    }
                    ServiceComponentKind::StreamData { .. } => {}
                }
            }
        }

        true
    }

    /// Feeds one batch of parsed FIG events (typically everything
    /// [`crate::fic::parse_fic`] returned for one FIC read) into the
    /// database. See this module's own doc comment for how this differs
    /// from the reference's callback-driven approach.
    pub fn apply(&mut self, events: &[FigEvent]) {
        for event in events {
            self.apply_one(event);
        }
    }

    fn apply_one(&mut self, event: &FigEvent) {
        match event {
            FigEvent::Fig0000(fig) => self.apply_fig_00_00(fig),
            FigEvent::Fig0001(fig) => self.apply_fig_00_01(fig),
            FigEvent::Fig0002(fig) => self.apply_fig_00_02(fig),
            FigEvent::Fig0003(fig) => self.apply_fig_00_03(fig),
            FigEvent::Fig0008(fig) => self.apply_fig_00_08(fig),
            FigEvent::Fig0009(fig) => self.apply_fig_00_09(fig),
            FigEvent::Fig0010(fig) => self.apply_fig_00_10(fig),
            FigEvent::Fig0013(fig) => self.apply_fig_00_13(fig),
            FigEvent::Fig0014(fig) => self.apply_fig_00_14(fig),
            FigEvent::Fig0017(fig) => self.apply_fig_00_17(fig),
            FigEvent::Fig0018(fig) => self.apply_fig_00_18(fig),
            FigEvent::Fig0019(fig) => self.apply_fig_00_19(fig),
            FigEvent::Fig0100(fig) => self.apply_fig_01_00(fig),
            FigEvent::Fig0101(fig) => self.apply_fig_01_01(fig),
            FigEvent::Fig0104(fig) => self.apply_fig_01_04(fig),
            FigEvent::Fig0105(fig) => self.apply_fig_01_05(fig),
            // FIG 0/4 (CA signalling only -- see its own doc comment in
            // fig_00.rs, nothing to store), 0/5 (service component
            // language -- the reference never wires up a handler for it
            // either), 0/6/0/21/0/24 (service linking / frequency info /
            // OE services -- feed Synthetic Service Following, a later
            // phase (`service_following.rs`); the reference's own handlers
            // for these three are commented-out stubs, so there is no
            // existing bookkeeping behaviour to mirror yet), 0/7
            // (configuration information -- parsed but never consumed by
            // the reference), 0/20
            // (Service Component Information -- not consumed by the
            // reference), 0/25/0/26 (OE announcement support/switching --
            // likewise), and 1/6 (X-PAD user application label -- the
            // reference's own FIG 1/6 handler is an empty function body)
            // carry no `DabEnsemble` bookkeeping to port.
            _ => {}
        }
    }

    /// `DabEnsemble::fig00_00_input`.
    fn apply_fig_00_00(&mut self, fig: &fig_00::Fig0000) {
        let is_initializing = self.ensemble_id != fig.ensemble_id;

        self.ensemble_id = fig.ensemble_id;
        self.is_alarm_announcement_supported = fig.is_alarm_supported;
        self.cif_count_high = fig.cif_count_high;
        self.cif_count_low = fig.cif_count_low;

        if (self.cif_count_high != self.cif_count_high_next
            || self.cif_count_low != self.cif_count_low_next)
            && !is_initializing
        {
            tracing::debug!(
                cif_high = self.cif_count_high,
                cif_low = self.cif_count_low,
                cif_high_next = self.cif_count_high_next,
                cif_low_next = self.cif_count_low_next,
                "CIF counter interrupted"
            );
        }

        self.cif_count_high_next = if self.cif_count_low_next >= 246 {
            (self.cif_count_high + 1) % 20
        } else {
            self.cif_count_high
        };
        self.cif_count_low_next = (self.cif_count_low + 4) % 250;
    }

    /// `DabEnsemble::fig00_01_input`.
    fn apply_fig_00_01(&mut self, sub: &fig_00::BasicSubchannelOrganization) {
        let protection_type = match sub.protection_type {
            fig_00::ProtectionType::Uep => 0,
            fig_00::ProtectionType::EepA8n => 1,
            fig_00::ProtectionType::EepB32n => 2,
        };

        if let Some(component) = self.stream_components.get_mut(&sub.sub_channel_id) {
            apply_subchannel_organization(component, sub, protection_type);
        } else {
            for component in self.packet_components.values_mut() {
                if component.subchannel_id == sub.sub_channel_id {
                    apply_subchannel_organization(component, sub, protection_type);
                }
            }
        }
    }

    /// `DabEnsemble::fig00_02_input`.
    fn apply_fig_00_02(&mut self, desc: &fig_00::ServiceDescription) {
        if self.services.contains_key(&desc.service_id) {
            return;
        }

        // Programme services carry only a 16-bit SId to begin with (see
        // `parse_fig_00_02`), so this mask is a no-op for them in practice
        // -- kept to mirror `fig00_02_input`'s explicit
        // `serviceId & 0x0000FFFF` for data-service parity.
        let service_id = if desc.is_programme_service {
            desc.service_id & 0x0000_FFFF
        } else {
            desc.service_id
        };

        let mut service = Service::new(service_id);
        service.is_programme_service = desc.is_programme_service;
        service.set_ca_id(desc.ca_id);
        service.num_service_components = desc.num_service_components;
        service.ensemble_frequency_khz = self.ensemble_frequency_khz;

        for comp in &desc.service_components {
            match comp.transport_mode_id {
                fig_00::Tmid::MscStreamAudio | fig_00::Tmid::MscStreamData => {
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        self.stream_components.entry(comp.sub_channel_id)
                    {
                        let kind = if comp.transport_mode_id == fig_00::Tmid::MscStreamAudio {
                            ServiceComponentKind::StreamAudio {
                                audio_service_component_type: comp.service_component_type,
                            }
                        } else {
                            ServiceComponentKind::StreamData {
                                data_service_component_type: comp.service_component_type,
                            }
                        };
                        let mut component = ServiceComponent::new(kind);
                        component.is_ca_applied = comp.is_ca_applied;
                        component.is_primary = comp.is_primary;
                        component.subchannel_id = comp.sub_channel_id;

                        e.insert(component);
                        service
                            .component_keys
                            .push(ComponentKey::Stream(comp.sub_channel_id));
                    }
                }
                fig_00::Tmid::MscPacketModeData => {
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        self.packet_components.entry(comp.service_component_id)
                    {
                        let mut component = ServiceComponent::new(ServiceComponentKind::PacketData {
                            service_component_id: comp.service_component_id,
                            data_service_component_type: crate::registered_tables::DataServiceComponentType::UnspecifiedData,
                            packet_address: 0xFFFF,
                            ca_organization: 0,
                            data_group_transport_used: false,
                        });
                        component.is_ca_applied = comp.is_ca_applied;
                        component.is_primary = comp.is_primary;

                        e.insert(component);
                    }
                    service
                        .component_keys
                        .push(ComponentKey::Packet(comp.service_component_id));
                }
                fig_00::Tmid::Reserved => {}
            }
        }

        self.services.insert(service_id, service);
    }

    /// `DabEnsemble::fig00_03_input`.
    fn apply_fig_00_03(&mut self, desc: &fig_00::PacketModeServiceComponentDescription) {
        if self.packet_components.is_empty() {
            return;
        }

        if let Some(component) = self.packet_components.get_mut(&desc.service_component_id) {
            component.subchannel_id = desc.subchannel_id;
            if let ServiceComponentKind::PacketData {
                data_service_component_type,
                packet_address,
                ca_organization,
                data_group_transport_used,
                ..
            } = &mut component.kind
            {
                *data_service_component_type = desc.data_service_component_type;
                *packet_address = desc.packet_address;
                *ca_organization = desc.ca_organization.unwrap_or(0);
                *data_group_transport_used = desc.data_group_transport_used;
            }
        }
    }

    /// `DabEnsemble::fig00_08_input`.
    fn apply_fig_00_08(&mut self, fig: &fig_00::ServiceComponentGlobalDefinition) {
        if self.stream_components.is_empty() {
            return;
        }

        if fig.is_short_form {
            if let Some(component) = self.stream_components.get_mut(&fig.subchannel_id) {
                component.set_component_id_within_service(fig.sc_id_s);
            }
        } else if let Some(component) = self.packet_components.get_mut(&fig.service_component_id) {
            component.set_component_id_within_service(fig.sc_id_s);
        }
    }

    /// `DabEnsemble::fig00_09_input`.
    fn apply_fig_00_09(&mut self, fig: &fig_00::Fig0009) {
        self.ensemble_ecc = fig.ensemble_ecc;
    }

    /// `DabEnsemble::fig00_10_input`.
    fn apply_fig_00_10(&mut self, fig: &fig_00::Fig0010) {
        self.date_time = Some(*fig);
    }

    /// `DabEnsemble::fig00_13_input`.
    fn apply_fig_00_13(&mut self, fig: &fig_00::UserApplicationInformation) {
        let Some(service) = self.services.get(&fig.service_id) else {
            return;
        };

        for &key in &service.component_keys {
            let component = match key {
                ComponentKey::Stream(id) => self.stream_components.get_mut(&id),
                ComponentKey::Packet(id) => self.packet_components.get_mut(&id),
            };
            let Some(component) = component else { continue };
            if component.component_id_within_service != fig.sc_id_s {
                continue;
            }

            for app in &fig.user_applications {
                component.add_user_application(UserApplication {
                    user_app_type: app.user_app_type,
                    is_ca_applied: app.ca_applied,
                    is_xpad_app: app.is_xpad_data,
                    xpad_app_type: app.xpad_app_type,
                    data_groups_used: app.data_groups_used,
                    data_service_component_type: app.data_service_component_type,
                    ca_organization: app.ca_organization,
                    user_app_data: app.user_app_data.clone(),
                });
            }
        }
    }

    /// `DabEnsemble::fig00_14_input`.
    fn apply_fig_00_14(&mut self, fig: &fig_00::FecSchemeDescription) {
        if self.packet_components.is_empty() || fig.fec_scheme != fig_00::FecScheme::Applied {
            return;
        }

        for component in self.packet_components.values_mut() {
            if component.subchannel_id == fig.sub_channel_id {
                component.is_fec_scheme_applied = true;
                break;
            }
        }
    }

    /// `DabEnsemble::fig00_17_input`.
    fn apply_fig_00_17(&mut self, fig: &fig_00::ProgrammeTypeInformation) {
        if let Some(service) = self.services.get_mut(&(fig.service_id as u32)) {
            service.programme_type_is_dynamic = fig.is_dynamic;
            service.set_programme_type_code(fig.int_pty_code);
        }
    }

    /// Not ported from `DabEnsemble::fig00_18_input` (its body is entirely
    /// commented out in the reference, so there's no bookkeeping behavior
    /// to mirror) -- new for Phase 6's `announcements.rs`, which needs to
    /// know a service's supported announcement types/clusters before
    /// deciding whether to switch to one.
    fn apply_fig_00_18(&mut self, fig: &fig_00::AnnouncementSupport) {
        self.announcement_supports
            .insert(fig.service_id, fig.clone());
    }

    /// `DabEnsemble::fig00_19_input`.
    // The two branches below do different things (conditional remove vs.
    // conditional insert), so this doesn't collapse into one `entry()` call
    // the way the `map_entry` lint suggests.
    #[allow(clippy::map_entry)]
    fn apply_fig_00_19(&mut self, fig: &fig_00::AnnouncementSwitching) {
        if self.active_announcements.contains_key(&fig.cluster_id) {
            if fig.announcements_switched.is_empty() {
                self.active_announcements.remove(&fig.cluster_id);
            }
        } else if fig.is_newly_introduced && !fig.announcements_switched.is_empty() {
            self.active_announcements
                .insert(fig.cluster_id, fig.clone());
        }
    }

    /// `DabEnsemble::fig01_00_input`.
    fn apply_fig_01_00(&mut self, fig: &fig_01::Fig0100) {
        if self.label.is_empty()
            && self.short_label.is_empty()
            && fig.ensemble_id == self.ensemble_id
        {
            self.label = fig.label.text.clone();
            self.short_label = fig.label.short_text.clone();
            self.label_charset = fig.charset;
        }
    }

    /// `DabEnsemble::fig01_01_input`.
    fn apply_fig_01_01(&mut self, fig: &fig_01::Fig0101) {
        if let Some(service) = self.services.get_mut(&(fig.programme_service_id as u32)) {
            service.set_label(fig.charset, &fig.label.text, &fig.label.short_text);
        }
    }

    /// `DabEnsemble::fig01_04_input`.
    fn apply_fig_01_04(&mut self, fig: &fig_01::Fig0104) {
        let Some(service) = self.services.get(&fig.service_id) else {
            return;
        };

        for &key in &service.component_keys {
            let component = match key {
                ComponentKey::Stream(id) => self.stream_components.get_mut(&id),
                ComponentKey::Packet(id) => self.packet_components.get_mut(&id),
            };
            let Some(component) = component else { continue };
            if component.component_id_within_service == fig.sc_id_s {
                component.set_label(fig.charset, &fig.label.text, &fig.label.short_text);
            }
        }
    }

    /// `DabEnsemble::fig01_05_input`.
    fn apply_fig_01_05(&mut self, fig: &fig_01::Fig0105) {
        if let Some(service) = self.services.get_mut(&fig.data_service_id) {
            service.set_label(fig.charset, &fig.label.text, &fig.label.short_text);
        }
    }
}

/// Shared by both branches of `fig00_01_input` (stream vs. packet
/// component lookup) -- the reference duplicates this same set of six
/// setter calls verbatim in each branch.
fn apply_subchannel_organization(
    component: &mut ServiceComponent,
    sub: &fig_00::BasicSubchannelOrganization,
    protection_type: u8,
) {
    component.msc_start_address = sub.start_address;
    component.subchannel_size = sub.sub_channel_size;
    component.convolutional_coding_rate = sub.convolutional_coding_rate.to_string();
    component.protection_level_string = sub.protection_level_type.label().to_string();
    component.protection_level = sub.protection_level;
    component.protection_type = protection_type;
    component.uep_table_index = sub.uep_table_index;
    component.subchannel_bitrate = sub.sub_channel_bitrate;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fic::fig::DabLabel;

    fn label(text: &str) -> DabLabel {
        DabLabel {
            text: text.to_string(),
            short_text: text.to_string(),
        }
    }

    /// Exercises the same FIG sequence a real ensemble collection would
    /// deliver for one service with one MSC-stream-audio component --
    /// FIG 0/0, 0/2 (creates the service+component), 0/1 (MSC placement),
    /// 0/8 (ScIdS), 0/9 (ECC), 1/0 (ensemble label), 1/1 (service label) --
    /// and checks the resulting database plus `is_collect_finished`.
    #[test]
    fn applies_full_fig_sequence_for_one_audio_service() {
        let mut ensemble = Ensemble::new();

        let events = vec![
            FigEvent::Fig0000(fig_00::Fig0000 {
                ensemble_id: 0x1234,
                change_flag: fig_00::ChangeFlag::NoChange,
                is_alarm_supported: false,
                cif_count_high: 0,
                cif_count_low: 0,
                occurrence_change: 0,
            }),
            FigEvent::Fig0002(fig_00::ServiceDescription {
                service_id: 0x1001,
                ca_id: 0,
                num_service_components: 1,
                is_programme_service: true,
                service_components: vec![fig_00::ServiceComponentDescription {
                    transport_mode_id: fig_00::Tmid::MscStreamAudio,
                    sub_channel_id: 5,
                    is_primary: true,
                    is_ca_applied: false,
                    service_component_type: 0,
                    service_component_id: 0xFFFF,
                }],
            }),
            FigEvent::Fig0001(fig_00::BasicSubchannelOrganization {
                is_programme_service: true,
                sub_channel_id: 5,
                start_address: 100,
                sub_channel_size: 72,
                sub_channel_bitrate: 128,
                convolutional_coding_rate: "3/4",
                protection_level_type: fig_00::ProtectionLevel::Level3A,
                protection_level: 2,
                protection_type: fig_00::ProtectionType::Uep,
                uep_table_index: 10,
            }),
            FigEvent::Fig0008(fig_00::ServiceComponentGlobalDefinition {
                is_programme_service: true,
                service_id: 0x1001,
                sc_id_s: 0,
                is_short_form: true,
                subchannel_id: 5,
                service_component_id: 0xFFFF,
            }),
            FigEvent::Fig0009(fig_00::Fig0009 {
                local_time_offset_half_hours: 2,
                ensemble_ecc: 0xE0,
                international_table_id: 1,
            }),
            FigEvent::Fig0100(fig_01::Fig0100 {
                ensemble_id: 0x1234,
                label: label("My Ensemble"),
                charset: CharacterSet::EbuLatin,
            }),
            FigEvent::Fig0101(fig_01::Fig0101 {
                programme_service_id: 0x1001,
                label: label("Test FM"),
                charset: CharacterSet::EbuLatin,
            }),
        ];

        ensemble.apply(&events);

        assert_eq!(ensemble.ensemble_id(), 0x1234);
        assert_eq!(ensemble.ensemble_ecc(), 0xE0);
        assert_eq!(ensemble.ensemble_label(), "My Ensemble");

        let service = ensemble
            .service(0x1001)
            .expect("service 0x1001 should exist");
        assert_eq!(service.label, "Test FM");
        assert_eq!(service.component_keys, vec![ComponentKey::Stream(5)]);

        let component = ensemble
            .component(ComponentKey::Stream(5))
            .expect("component should exist");
        assert_eq!(component.msc_start_address, 100);
        assert_eq!(component.subchannel_size, 72);
        assert!(component.is_primary);
        assert_eq!(component.component_id_within_service, 0);
        assert!(matches!(
            component.kind,
            ServiceComponentKind::StreamAudio { .. }
        ));

        assert!(ensemble.is_collect_finished());
    }

    #[test]
    fn is_collect_finished_false_before_any_fic_data() {
        let ensemble = Ensemble::new();
        assert!(!ensemble.is_collect_finished());
    }

    #[test]
    fn reset_clears_state_but_keeps_frequency() {
        let mut ensemble = Ensemble::new();
        ensemble.set_frequency_khz(174_928);
        ensemble.apply(&[FigEvent::Fig0009(fig_00::Fig0009 {
            local_time_offset_half_hours: 0,
            ensemble_ecc: 0xE0,
            international_table_id: 1,
        })]);
        assert_eq!(ensemble.ensemble_ecc(), 0xE0);

        ensemble.reset();

        assert_eq!(ensemble.ensemble_ecc(), 0xFF);
        assert_eq!(ensemble.ensemble_id(), 0xFFFF);
        assert_eq!(ensemble.services().count(), 0);
        assert_eq!(ensemble.frequency_khz(), 174_928);
    }
}
