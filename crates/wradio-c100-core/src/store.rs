//! Persisted station database (scan results / favorites) + presets.
//!
//! New functionality, not a port of existing C++ -- the reference has no
//! equivalent of this (it hardcoded one PPP-RTK data service, the whole
//! use case this crate drops, see `RUST_PORT_PROMPT.md`'s intro). Designed
//! per `RUST_PORT_PROMPT.md`'s "Favorites, presets & service following"
//! section, following the shape [DABdream+](https://xdaforums.com/t/dabdream-web-radio-real-dab-dab-player.4638309/)
//! (a mature Android DAB player for this same class of generic USB
//! dongle) uses per its own FAQ, as that section names.
//!
//! The core crate persists via a plain `serde_json` file, per
//! `RUST_PORT_PROMPT.md`: "The core crate itself (for the headless
//! example) can do the equivalent [of `tauri-plugin-store`] with a plain
//! serde_json file" -- the Tauri plugin crate (a later phase) uses
//! `tauri-plugin-store` instead, over the same `StationRecord`/`Preset`
//! DTOs (already `Serialize`/`Deserialize` for exactly that reuse).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Uniquely identifies one station: the ensemble it was found on plus its
/// service id within that ensemble. A service id alone isn't globally
/// unique -- the same id can legitimately appear in different ensembles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationRef {
    pub ensemble_id: u16,
    pub service_id: u32,
}

/// One station's persisted record: latest-known metadata plus every
/// frequency it's been found on.
///
/// `frequencies_khz` are Synthetic Service Following (SSF) candidates, in
/// preference order (index 0 tried first) -- see `service_following.rs`
/// for how that ordering changes over time ("persistent instance switch").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationRecord {
    pub station: StationRef,
    pub label: String,
    pub programme_type_code: u8,
    /// Whether this service is actually playable -- a DAB+ (ASCTY 0x3F)
    /// audio stream component with a resolved subchannel/bitrate, per
    /// [`crate::ensemble::Ensemble::service_has_dab_plus_audio`]. Lets a
    /// caller (e.g. the Tauri plugin's station browser) filter out
    /// data-only services (TPEG, etc.) and classic MPEG audio without
    /// needing to re-derive this from raw component data itself.
    pub has_dab_plus_audio: bool,
    pub frequencies_khz: Vec<u32>,
    pub last_seen_unix: i64,
}

/// One user-assignable preset slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub slot: u32,
    pub station: StationRef,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreData {
    stations: Vec<StationRecord>,
    presets: Vec<Preset>,
    /// Auto-tune-on-startup target: the most recently played station.
    last_played: Option<StationRef>,
}

/// Persisted station database + presets, backed by a `serde_json` file.
pub struct Store {
    path: PathBuf,
    data: StoreData,
}

impl Store {
    /// Loads the store from `path`. A missing or corrupt file starts an
    /// empty store rather than erroring -- this is a scan-result cache,
    /// not critical data, and any of it is recoverable by rescanning, so
    /// a bad load shouldn't block the tuner from starting.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let data = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { path, data }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Persists the current state to disk, creating the parent directory
    /// if needed.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&self.data)?;
        std::fs::write(&self.path, bytes)?;
        Ok(())
    }

    /// Upserts a station found during a scan: refreshes label/programme
    /// type and adds `frequency_khz` as a candidate (if not already
    /// known) for an existing entry, or inserts a new one. Matches
    /// `RUST_PORT_PROMPT.md`'s "update label/PTY/frequency, but don't
    /// delete an entry just because this particular scan pass didn't
    /// reach it" -- entries are only ever added to or refreshed by
    /// scanning, never removed (weak/marginal reception dropping a
    /// station from one scan pass is expected, not a reason to forget it).
    ///
    /// A no-op if `station.ensemble_id == 0xFFFF`: that's the sentinel
    /// [`crate::ensemble::Ensemble`] itself starts at and uses for "not
    /// yet known from FIG 0/0" (mirroring the reference's own
    /// `m_ensembleId{0xFFFF}` default) -- a caller polling the FIC can
    /// otherwise learn a service's id (FIG 0/2) before its ensemble's id
    /// (FIG 0/0) within a bounded collection window (a real gap found
    /// scanning real hardware: a short per-channel scan timeout caught
    /// FIG 0/2 but not FIG 0/0 for one ensemble), and upserting under that
    /// bogus ref would create permanent duplicate entries once the real
    /// ensemble id is later learned. Guarded here rather than only in
    /// callers so every current and future caller gets this for free.
    pub fn upsert_station(
        &mut self,
        station: StationRef,
        label: &str,
        programme_type_code: u8,
        has_dab_plus_audio: bool,
        frequency_khz: u32,
    ) {
        if station.ensemble_id == 0xFFFF {
            return;
        }
        let now = now_unix();
        if let Some(existing) = self.data.stations.iter_mut().find(|s| s.station == station) {
            if !label.is_empty() {
                existing.label = label.to_string();
            }
            existing.programme_type_code = programme_type_code;
            existing.has_dab_plus_audio = has_dab_plus_audio;
            if !existing.frequencies_khz.contains(&frequency_khz) {
                existing.frequencies_khz.push(frequency_khz);
            }
            existing.last_seen_unix = now;
        } else {
            self.data.stations.push(StationRecord {
                station,
                label: label.to_string(),
                programme_type_code,
                has_dab_plus_audio,
                frequencies_khz: vec![frequency_khz],
                last_seen_unix: now,
            });
        }
    }

    /// Reorders `station`'s frequency candidates so `frequency_khz` is
    /// tried first next time -- the "persistent instance switch" from
    /// `RUST_PORT_PROMPT.md`: once (synthetic or FIG 0/21-driven) service
    /// following switches to a better-received frequency, remember it as
    /// the new default rather than re-testing the original weak one on
    /// every subsequent playback. A no-op if `frequency_khz` isn't
    /// already a known candidate for this station.
    pub fn promote_frequency(&mut self, station: StationRef, frequency_khz: u32) {
        if let Some(existing) = self.data.stations.iter_mut().find(|s| s.station == station) {
            if let Some(pos) = existing
                .frequencies_khz
                .iter()
                .position(|&f| f == frequency_khz)
            {
                existing.frequencies_khz.remove(pos);
                existing.frequencies_khz.insert(0, frequency_khz);
            }
        }
    }

    pub fn stations(&self) -> &[StationRecord] {
        &self.data.stations
    }

    pub fn station(&self, station: StationRef) -> Option<&StationRecord> {
        self.data.stations.iter().find(|s| s.station == station)
    }

    /// Records `station` as the auto-tune-on-startup target.
    pub fn set_last_played(&mut self, station: StationRef) {
        self.data.last_played = Some(station);
    }

    pub fn last_played(&self) -> Option<StationRef> {
        self.data.last_played
    }

    pub fn list_presets(&self) -> &[Preset] {
        &self.data.presets
    }

    pub fn preset(&self, slot: u32) -> Option<StationRef> {
        self.data
            .presets
            .iter()
            .find(|p| p.slot == slot)
            .map(|p| p.station)
    }

    pub fn set_preset(&mut self, slot: u32, station: StationRef) {
        if let Some(existing) = self.data.presets.iter_mut().find(|p| p.slot == slot) {
            existing.station = station;
        } else {
            self.data.presets.push(Preset { slot, station });
        }
    }

    pub fn clear_preset(&mut self, slot: u32) {
        self.data.presets.retain(|p| p.slot != slot);
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(ensemble_id: u16, service_id: u32) -> StationRef {
        StationRef {
            ensemble_id,
            service_id,
        }
    }

    #[test]
    fn upsert_ignores_the_unknown_ensemble_id_sentinel() {
        let mut store = Store::open("/nonexistent/path/for/sentinel/test.json");
        store.upsert_station(station(0xFFFF, 0xE1), "Ghost", 0, true, 227_360);
        assert!(
            store.stations().is_empty(),
            "0xFFFF ensemble id must never be persisted"
        );
    }

    #[test]
    fn upsert_creates_then_refreshes_without_duplicating_frequencies() {
        let mut store = Store::open("/nonexistent/path/that/should/just/start/empty.json");
        let s = station(0x8001, 0xE1);

        store.upsert_station(s, "NPO Radio 1", 10, true, 227_360);
        assert_eq!(store.stations().len(), 1);
        assert_eq!(store.station(s).unwrap().frequencies_khz, vec![227_360]);

        // A second scan hit on the same frequency shouldn't duplicate it;
        // a hit on a different frequency should be added as another SSF
        // candidate.
        store.upsert_station(s, "NPO Radio 1", 10, true, 227_360);
        store.upsert_station(s, "NPO Radio 1", 10, true, 174_928);
        let rec = store.station(s).unwrap();
        assert_eq!(rec.frequencies_khz, vec![227_360, 174_928]);
        assert_eq!(
            store.stations().len(),
            1,
            "still one station, not a duplicate entry"
        );
    }

    #[test]
    fn upsert_never_forgets_a_station_not_seen_this_scan() {
        let mut store = Store::open("/nonexistent/path/for/this/test.json");
        let a = station(0x8001, 0xE1);
        let b = station(0x8001, 0xE2);
        store.upsert_station(a, "A", 0, true, 227_360);
        store.upsert_station(b, "B", 0, true, 227_360);

        // Simulate a rescan that only found `a` again -- `b` must remain.
        store.upsert_station(a, "A", 0, true, 227_360);
        assert_eq!(store.stations().len(), 2);
        assert!(store.station(b).is_some());
    }

    #[test]
    fn promote_frequency_reorders_candidates() {
        let mut store = Store::open("/nonexistent/path/for/promote/test.json");
        let s = station(0x8001, 0xE1);
        store.upsert_station(s, "A", 0, true, 100);
        store.upsert_station(s, "A", 0, true, 200);
        store.upsert_station(s, "A", 0, true, 300);
        assert_eq!(
            store.station(s).unwrap().frequencies_khz,
            vec![100, 200, 300]
        );

        store.promote_frequency(s, 300);
        assert_eq!(
            store.station(s).unwrap().frequencies_khz,
            vec![300, 100, 200]
        );

        // Promoting an unknown frequency is a no-op, not an error.
        store.promote_frequency(s, 999);
        assert_eq!(
            store.station(s).unwrap().frequencies_khz,
            vec![300, 100, 200]
        );
    }

    #[test]
    fn presets_set_list_clear() {
        let mut store = Store::open("/nonexistent/path/for/preset/test.json");
        let a = station(1, 1);
        let b = station(1, 2);

        store.set_preset(1, a);
        store.set_preset(2, b);
        assert_eq!(store.list_presets().len(), 2);
        assert_eq!(store.preset(1), Some(a));

        // Re-assigning a slot overwrites rather than duplicating it.
        store.set_preset(1, b);
        assert_eq!(store.list_presets().len(), 2);
        assert_eq!(store.preset(1), Some(b));

        store.clear_preset(1);
        assert_eq!(store.list_presets().len(), 1);
        assert_eq!(store.preset(1), None);
    }

    #[test]
    fn save_and_reload_round_trips() {
        let dir =
            std::env::temp_dir().join(format!("wradio-c100-store-test-{}", std::process::id()));
        let path = dir.join("stations.json");
        let _ = std::fs::remove_dir_all(&dir);

        let s = station(0x8001, 0xE1);
        {
            let mut store = Store::open(&path);
            store.upsert_station(s, "NPO Radio 1", 10, true, 227_360);
            store.set_preset(1, s);
            store.set_last_played(s);
            store
                .save()
                .expect("save should succeed, creating the parent dir");
        }

        let reloaded = Store::open(&path);
        assert_eq!(reloaded.stations().len(), 1);
        assert_eq!(reloaded.station(s).unwrap().label, "NPO Radio 1");
        assert_eq!(reloaded.preset(1), Some(s));
        assert_eq!(reloaded.last_played(), Some(s));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
