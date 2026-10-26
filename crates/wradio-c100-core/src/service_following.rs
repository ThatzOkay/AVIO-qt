//! Synthetic Service Following (SSF): the runtime "switch away from a
//! degraded candidate" policy layered on top of `store.rs`'s
//! candidate-frequency bookkeeping.
//!
//! New functionality -- there's no reference C++ equivalent to port (see
//! `store.rs`'s doc comment for why). Grouping same-service-id scan hits
//! into one station with multiple frequency candidates, and remembering a
//! persistent instance switch, are `store.rs`'s job
//! (`Store::upsert_station`/`promote_frequency`); this module is the
//! runtime decision on top of that: given a station's candidate list and
//! the currently-tuned candidate's signal quality, decide whether (and to
//! what) to switch. Matches `RUST_PORT_PROMPT.md`'s "Favorites, presets &
//! service following" section -- this is the scan-result heuristic (match
//! on service id) it describes as simpler than and distinct from acting
//! on the FIC's own FIG 0/21 frequency information or FIG 0/6 service
//! linking (both parsed already, see `ensemble.rs`, just not acted on).

use std::time::Duration;

use crate::error::Result;
use crate::store::{StationRef, Store};
use crate::tuner::{SignalQuality, Tuner};

/// Below this signal-bar level (0..=6, see [`crate::tuner::SignalQuality`]),
/// a candidate is considered degraded enough to be worth switching away
/// from. `RUST_PORT_PROMPT.md` doesn't specify a numeric threshold here
/// (SSF is new functionality, not a port) -- 1 is chosen as "barely above
/// the no-signal floor", conservative enough not to flap between
/// candidates on ordinary reception noise. Construct a [`ServiceFollowing`]
/// with a different value if that's not the right call for a given
/// deployment.
pub const DEFAULT_DEGRADED_THRESHOLD: u8 = 1;

/// SSF switch-away policy.
pub struct ServiceFollowing {
    degraded_threshold: u8,
}

impl Default for ServiceFollowing {
    fn default() -> Self {
        Self::new(DEFAULT_DEGRADED_THRESHOLD)
    }
}

impl ServiceFollowing {
    pub fn new(degraded_threshold: u8) -> Self {
        Self { degraded_threshold }
    }

    /// Given `candidates` (a station's known frequencies, in preference
    /// order -- see `store::StationRecord::frequencies_khz`) and the
    /// signal quality of the currently-tuned `current_khz`, returns the
    /// next candidate worth trying if `current_khz` looks degraded and a
    /// different candidate exists, or `None` if the current candidate is
    /// fine as-is (or there's no alternative to switch to).
    pub fn next_candidate(
        &self,
        candidates: &[u32],
        current_khz: u32,
        quality: SignalQuality,
    ) -> Option<u32> {
        let is_degraded = !quality.locked
            || quality
                .signal_bars
                .is_none_or(|bars| bars < self.degraded_threshold);
        if !is_degraded {
            return None;
        }

        candidates.iter().copied().find(|&khz| khz != current_khz)
    }
}

/// Tunes to `station`'s best candidate: tries each known frequency (in
/// the store's current preference order) until one achieves lock,
/// promoting it via [`Store::promote_frequency`] ("persistent instance
/// switch") if it isn't already the first choice. Returns the frequency
/// ultimately tuned to, or `None` if `station` is unknown to `store` or
/// none of its candidates locked.
pub async fn tune_best_candidate(
    tuner: &mut Tuner,
    store: &mut Store,
    station: StationRef,
    lock_timeout: Duration,
) -> Result<Option<u32>> {
    let Some(record) = store.station(station) else {
        return Ok(None);
    };
    let candidates = record.frequencies_khz.clone();

    for khz in candidates {
        if tuner.tune_and_wait_for_lock(khz, lock_timeout, 1).await? {
            store.promote_frequency(station, khz);
            return Ok(Some(khz));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quality(locked: bool, signal_bars: Option<u8>) -> SignalQuality {
        SignalQuality {
            locked,
            signal_bars,
            muted: false,
            device_connected: true,
        }
    }

    #[test]
    fn stays_on_current_candidate_when_signal_is_fine() {
        let sf = ServiceFollowing::default();
        let candidates = [100, 200, 300];
        assert_eq!(
            sf.next_candidate(&candidates, 100, quality(true, Some(3))),
            None
        );
    }

    #[test]
    fn switches_to_next_candidate_when_unlocked() {
        let sf = ServiceFollowing::default();
        let candidates = [100, 200, 300];
        assert_eq!(
            sf.next_candidate(&candidates, 100, quality(false, None)),
            Some(200)
        );
    }

    #[test]
    fn switches_to_next_candidate_when_signal_below_threshold() {
        let sf = ServiceFollowing::new(2);
        let candidates = [100, 200, 300];
        assert_eq!(
            sf.next_candidate(&candidates, 100, quality(true, Some(1))),
            Some(200)
        );
        assert_eq!(
            sf.next_candidate(&candidates, 100, quality(true, Some(2))),
            None
        );
    }

    #[test]
    fn skips_the_currently_tuned_candidate_even_if_listed_first() {
        let sf = ServiceFollowing::default();
        // Current candidate (200) isn't index 0 -- must still be skipped,
        // not just "skip index 0".
        let candidates = [200, 300];
        assert_eq!(
            sf.next_candidate(&candidates, 200, quality(false, None)),
            Some(300)
        );
    }

    #[test]
    fn no_alternative_candidate_returns_none_even_when_degraded() {
        let sf = ServiceFollowing::default();
        let candidates = [100];
        assert_eq!(
            sf.next_candidate(&candidates, 100, quality(false, None)),
            None
        );
    }
}
