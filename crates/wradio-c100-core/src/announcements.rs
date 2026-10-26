//! FIG 0/18 (announcement support) + FIG 0/19 (announcement switching)
//! driven subchannel switching -- DAB's RDS-TA equivalent. New
//! functionality, not a port of existing C++ (see `store.rs`'s doc
//! comment for why no reference equivalent exists to port).
//!
//! Remembers the hardware constraint `RUST_PORT_PROMPT.md` calls out:
//! `MAXIMUM_CONCURRENT_SUBCHANNELS = 1` -- this chip can only decode one
//! subchannel at a time, so an announcement genuinely interrupts the
//! primary programme (switching the open subchannel) for its duration,
//! rather than ducking under it the way a receiver with dedicated
//! per-stream decode hardware could.

use crate::ensemble::Ensemble;
use crate::registered_tables::AnnouncementType;

/// An announcement has started or ended, and which subchannel to switch
/// to/back to as a result. The caller is responsible for actually
/// commanding the switch (`Tuner::close_subchannel`/`open_subchannel`)
/// and updating any playback pipeline for the new subchannel -- this type
/// only carries the *decision*.
///
/// `#[serde(tag = "type")]` (internally tagged: `{"type":"started",...}`)
/// rather than serde's default externally-tagged shape
/// (`{"started":{...}}`) -- picked deliberately for the plugin's JS side
/// (`tauri-plugin-wradio-c100/guest-js/index.ts`'s `AnnouncementEvent`
/// type), where a `type` discriminant field is the idiomatic TypeScript
/// discriminated-union shape (`event.type === "started"`) instead of
/// needing an `"started" in event` presence check.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AnnouncementEvent {
    /// Switch to `subchannel_id` for the duration of `announcement_type`.
    Started {
        announcement_type: AnnouncementType,
        subchannel_id: u8,
    },
    /// The announcement ended; switch back to `subchannel_id` (the
    /// primary programme).
    Ended { subchannel_id: u8 },
}

/// Tracks whether the currently-open subchannel is the listener's primary
/// programme or an announcement interruption, and decides when to switch
/// between them.
pub struct AnnouncementSwitcher {
    /// The subchannel the listener actually selected; restored once any
    /// announcement interruption ends.
    primary_subchannel_id: u8,
    /// The cluster id currently interrupting the primary programme, if
    /// any.
    active_cluster_id: Option<u8>,
}

impl AnnouncementSwitcher {
    pub fn new(primary_subchannel_id: u8) -> Self {
        Self {
            primary_subchannel_id,
            active_cluster_id: None,
        }
    }

    /// The subchannel to be playing when no announcement is interrupting
    /// it.
    pub fn primary_subchannel_id(&self) -> u8 {
        self.primary_subchannel_id
    }

    /// Whether an announcement is currently interrupting the primary
    /// programme.
    pub fn is_interrupted(&self) -> bool {
        self.active_cluster_id.is_some()
    }

    /// Call whenever `ensemble`'s state has just been updated (e.g. after
    /// `Ensemble::apply`) for the service currently selected
    /// (`primary_service_id`). Returns the subchannel to switch to and
    /// the event to surface (to a UI/plugin layer), if a change is
    /// needed this call; `None` if nothing changed (either still on the
    /// primary programme with nothing relevant active, or still mid-
    /// announcement with that same announcement still active).
    ///
    /// Once interrupting for a cluster, stays on it until that *specific*
    /// cluster is no longer listed as active -- deliberately not jumping
    /// straight from one announcement into a different one that happens
    /// to become relevant in the meantime; always returns to the primary
    /// programme first.
    pub fn update(
        &mut self,
        ensemble: &Ensemble,
        primary_service_id: u32,
    ) -> Option<(u8, AnnouncementEvent)> {
        if let Some(cluster_id) = self.active_cluster_id {
            if ensemble
                .active_announcements()
                .any(|a| a.cluster_id == cluster_id)
            {
                return None;
            }
            self.active_cluster_id = None;
            return Some((
                self.primary_subchannel_id,
                AnnouncementEvent::Ended {
                    subchannel_id: self.primary_subchannel_id,
                },
            ));
        }

        // Idle: a relevant announcement is one whose cluster the primary
        // service signals support for (FIG 0/18) and whose switched-in
        // types include at least one the service supports.
        let support = ensemble.announcement_support(primary_service_id as u16)?;
        let active = ensemble.active_announcements().find(|a| {
            support.cluster_ids.contains(&a.cluster_id)
                && a.announcements_switched
                    .iter()
                    .any(|t| support.supported_announcements.contains(t))
        })?;

        self.active_cluster_id = Some(active.cluster_id);
        let announcement_type = *active.announcements_switched.first()?;
        Some((
            active.subchannel_id,
            AnnouncementEvent::Started {
                announcement_type,
                subchannel_id: active.subchannel_id,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fic::{fig_00, FigEvent};

    const SERVICE_ID: u32 = 0x1001;
    const PRIMARY_SUBCHAN: u8 = 3;
    const ANNOUNCEMENT_SUBCHAN: u8 = 9;
    const CLUSTER_ID: u8 = 5;

    fn support_event() -> FigEvent {
        FigEvent::Fig0018(fig_00::AnnouncementSupport {
            service_id: SERVICE_ID as u16,
            supported_announcements: vec![AnnouncementType::RoadTrafficFlash],
            cluster_ids: vec![CLUSTER_ID],
        })
    }

    fn switching_event(announcements_switched: Vec<AnnouncementType>) -> FigEvent {
        FigEvent::Fig0019(fig_00::AnnouncementSwitching {
            cluster_id: CLUSTER_ID,
            announcements_switched,
            is_newly_introduced: true,
            subchannel_id: ANNOUNCEMENT_SUBCHAN,
        })
    }

    #[test]
    fn starts_and_ends_around_a_relevant_announcement() {
        let mut ensemble = Ensemble::new();
        ensemble.apply(&[support_event()]);

        let mut switcher = AnnouncementSwitcher::new(PRIMARY_SUBCHAN);
        assert_eq!(
            switcher.update(&ensemble, SERVICE_ID),
            None,
            "nothing active yet"
        );
        assert!(!switcher.is_interrupted());

        ensemble.apply(&[switching_event(vec![AnnouncementType::RoadTrafficFlash])]);
        let (subchan, event) = switcher
            .update(&ensemble, SERVICE_ID)
            .expect("announcement should start");
        assert_eq!(subchan, ANNOUNCEMENT_SUBCHAN);
        assert_eq!(
            event,
            AnnouncementEvent::Started {
                announcement_type: AnnouncementType::RoadTrafficFlash,
                subchannel_id: ANNOUNCEMENT_SUBCHAN
            }
        );
        assert!(switcher.is_interrupted());

        // Still active: no further change.
        assert_eq!(switcher.update(&ensemble, SERVICE_ID), None);

        // Announcement ends (empty switched list clears it from
        // `active_announcements`, see `Ensemble::apply_fig_00_19`).
        ensemble.apply(&[switching_event(vec![])]);
        let (subchan, event) = switcher
            .update(&ensemble, SERVICE_ID)
            .expect("announcement should end");
        assert_eq!(subchan, PRIMARY_SUBCHAN);
        assert_eq!(
            event,
            AnnouncementEvent::Ended {
                subchannel_id: PRIMARY_SUBCHAN
            }
        );
        assert!(!switcher.is_interrupted());
    }

    #[test]
    fn irrelevant_announcement_type_is_ignored() {
        let mut ensemble = Ensemble::new();
        ensemble.apply(&[support_event()]); // supports RoadTrafficFlash only

        ensemble.apply(&[FigEvent::Fig0019(fig_00::AnnouncementSwitching {
            cluster_id: CLUSTER_ID,
            announcements_switched: vec![AnnouncementType::SportReport], // not supported
            is_newly_introduced: true,
            subchannel_id: ANNOUNCEMENT_SUBCHAN,
        })]);

        let mut switcher = AnnouncementSwitcher::new(PRIMARY_SUBCHAN);
        assert_eq!(switcher.update(&ensemble, SERVICE_ID), None);
        assert!(!switcher.is_interrupted());
    }

    #[test]
    fn service_with_no_announcement_support_never_interrupts() {
        let ensemble = Ensemble::new(); // no FIG 0/18 applied at all
        let mut switcher = AnnouncementSwitcher::new(PRIMARY_SUBCHAN);
        assert_eq!(switcher.update(&ensemble, SERVICE_ID), None);
    }
}
