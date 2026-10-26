//! Navigation status channel handler (`CH.NAVIGATION` = 12).
//!
//! Phone -> HU only. Carries Google Maps turn-by-turn data so a HU with a cluster/side widget
//! can show maneuver + distance independently of the main video stream.
//!
//! Message IDs (NavigationStatusMessageId.proto):
//!   0x8001 INSTRUMENT_CLUSTER_START               (StatusStart, empty)
//!   0x8002 INSTRUMENT_CLUSTER_STOP                (StatusStop,  empty)
//!   0x8003 INSTRUMENT_CLUSTER_NAVIGATION_STATUS         (NavigationStatus.status enum)
//!   0x8004 INSTRUMENT_CLUSTER_NAVIGATION_TURN_EVENT     [deprecated]
//!   0x8005 INSTRUMENT_CLUSTER_NAVIGATION_DISTANCE_EVENT [deprecated]
//!   0x8006 INSTRUMENT_CLUSTER_NAVIGATION_STATE          (steps + destinations)
//!   0x8007 INSTRUMENT_CLUSTER_NAVIGATION_CURRENT_POSITION
//!
//! The deprecated TURN_EVENT / DISTANCE_EVENT pair is what current Maps actually sends —
//! modern STATE/CURRENT_POSITION are reserved for cluster apps that aasdk hosts don't
//! typically implement.

use super::proto_enc::{decode_fields, decode_varint_value};

pub mod nav_msg {
    pub const START_INDICATION: u16 = 0x8001;
    pub const STOP_INDICATION: u16 = 0x8002;
    pub const STATUS: u16 = 0x8003;
    pub const TURN_EVENT: u16 = 0x8004;
    pub const DISTANCE_EVENT: u16 = 0x8005;
    pub const STATE: u16 = 0x8006;
    pub const CURRENT_POSITION: u16 = 0x8007;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavigationState {
    #[default]
    Unavailable,
    Active,
    Inactive,
    Rerouting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationTurnSide {
    Left,
    Right,
    Unspecified,
}

/// NextTurnEnum from NavigationNextTurnEvent.proto (deprecated TURN_EVENT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationTurnEvent {
    Unknown,
    Depart,
    NameChange,
    SlightTurn,
    Turn,
    SharpTurn,
    UTurn,
    OnRamp,
    OffRamp,
    Fork,
    Merge,
    RoundaboutEnter,
    RoundaboutExit,
    RoundaboutEnterAndExit,
    Straight,
    FerryBoat,
    FerryTrain,
    Destination,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NavigationStatusUpdate {
    pub state: NavigationState,
}

#[derive(Debug, Clone, Default)]
pub struct NavigationTurnUpdate {
    pub road: Option<String>,
    pub turn_side: Option<NavigationTurnSide>,
    pub event: Option<NavigationTurnEvent>,
    /// Raw turn-icon image bytes (PNG/bitmap).
    pub image: Option<Vec<u8>>,
    pub turn_number: Option<u32>,
    pub turn_angle: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NavigationDistanceUpdate {
    pub distance_meters: u32,
    pub time_to_turn_seconds: u32,
    /// Display value x1000 in the unit indicated by `display_unit` (e.g. 1.5 km = 1500).
    pub display_distance_e3: Option<u32>,
    pub display_unit: Option<u32>,
}

/// Modern NavigationState (STATE, AA >= 1.7): current step + destination.
#[derive(Debug, Clone, Default)]
pub struct NavigationStateUpdate {
    /// NavigationManeuver.NavigationType enum of the current step.
    pub maneuver_type: Option<u32>,
    pub road_name: Option<String>,
    pub cue: Option<String>,
    pub destination_address: Option<String>,
}

/// Modern NavigationCurrentPosition (CURRENT_POSITION, AA >= 1.7): live distances.
#[derive(Debug, Clone, Default)]
pub struct NavigationPositionUpdate {
    pub step_distance_meters: Option<u32>,
    pub step_distance_display: Option<String>,
    pub time_to_step_seconds: Option<u32>,
    pub destination_meters: Option<u32>,
    pub destination_display: Option<String>,
    /// NavigationDistance.DistanceUnits enum.
    pub destination_units: Option<u32>,
    /// Clock time of arrival, e.g. "21:58".
    pub eta_text: Option<String>,
    pub time_to_arrival_seconds: Option<u32>,
    pub current_road_name: Option<String>,
}

#[derive(Debug)]
pub enum NavEvent {
    Start,
    Stop,
    Status(NavigationStatusUpdate),
    Turn(NavigationTurnUpdate),
    Distance(NavigationDistanceUpdate),
    State(NavigationStateUpdate),
    Position(NavigationPositionUpdate),
    None,
}

pub fn handle_message(msg_id: u16, payload: &[u8]) -> NavEvent {
    match msg_id {
        nav_msg::START_INDICATION => NavEvent::Start,
        nav_msg::STOP_INDICATION => NavEvent::Stop,
        nav_msg::STATUS => NavEvent::Status(decode_status(payload)),
        nav_msg::TURN_EVENT => NavEvent::Turn(decode_turn_event(payload)),
        nav_msg::DISTANCE_EVENT => NavEvent::Distance(decode_distance_event(payload)),
        nav_msg::STATE => NavEvent::State(decode_state(payload)),
        nav_msg::CURRENT_POSITION => NavEvent::Position(decode_position(payload)),
        _ => NavEvent::None,
    }
}

fn decode_status(payload: &[u8]) -> NavigationStatusUpdate {
    let mut raw = 0;
    for f in decode_fields(payload) {
        if f.field == 1 && f.wire == 0 {
            raw = decode_varint_value(&f.bytes);
        }
    }
    let state = match raw {
        1 => NavigationState::Active,
        2 => NavigationState::Inactive,
        3 => NavigationState::Rerouting,
        _ => NavigationState::Unavailable,
    };
    NavigationStatusUpdate { state }
}

fn decode_turn_event(payload: &[u8]) -> NavigationTurnUpdate {
    let mut out = NavigationTurnUpdate::default();
    for f in decode_fields(payload) {
        match f.field {
            1 => out.road = Some(String::from_utf8_lossy(&f.bytes).into_owned()),
            2 => {
                // turn_side (TurnSide enum: 1=LEFT, 2=RIGHT, 3=UNSPECIFIED)
                let v = decode_varint_value(&f.bytes);
                out.turn_side = Some(match v {
                    1 => NavigationTurnSide::Left,
                    2 => NavigationTurnSide::Right,
                    _ => NavigationTurnSide::Unspecified,
                });
            }
            3 => out.event = Some(map_next_turn_enum(decode_varint_value(&f.bytes))),
            4 => out.image = Some(f.bytes),
            5 => out.turn_number = Some(decode_varint_value(&f.bytes)),
            6 => out.turn_angle = Some(decode_varint_value(&f.bytes)),
            _ => {}
        }
    }
    out
}

fn decode_distance_event(payload: &[u8]) -> NavigationDistanceUpdate {
    let mut out = NavigationDistanceUpdate::default();
    for f in decode_fields(payload) {
        match f.field {
            1 => out.distance_meters = decode_varint_value(&f.bytes),
            2 => out.time_to_turn_seconds = decode_varint_value(&f.bytes),
            3 => out.display_distance_e3 = Some(decode_varint_value(&f.bytes)),
            4 => out.display_unit = Some(decode_varint_value(&f.bytes)),
            _ => {}
        }
    }
    out
}

// NavigationState { steps=1 (NavigationStep), destinations=2 (NavigationDestination) }
fn decode_state(payload: &[u8]) -> NavigationStateUpdate {
    let mut out = NavigationStateUpdate::default();
    for f in decode_fields(payload) {
        if f.field == 1 && f.wire == 2 && out.maneuver_type.is_none() && out.road_name.is_none() {
            // first NavigationStep { maneuver=1, road=2, lanes=3, cue=4 }
            for s in decode_fields(&f.bytes) {
                if s.field == 1 && s.wire == 2 {
                    // NavigationManeuver { type=1 }
                    for m in decode_fields(&s.bytes) {
                        if m.field == 1 && m.wire == 0 {
                            out.maneuver_type = Some(decode_varint_value(&m.bytes));
                        }
                    }
                } else if s.field == 2 && s.wire == 2 {
                    // NavigationRoad { name=1 }
                    for r in decode_fields(&s.bytes) {
                        if r.field == 1 && r.wire == 2 {
                            out.road_name = Some(String::from_utf8_lossy(&r.bytes).into_owned());
                        }
                    }
                } else if s.field == 4 && s.wire == 2 && out.cue.is_none() {
                    // NavigationCue { alternate_text=1 (repeated) } — take the first.
                    for c in decode_fields(&s.bytes) {
                        if c.field == 1 && c.wire == 2 && out.cue.is_none() {
                            out.cue = Some(String::from_utf8_lossy(&c.bytes).into_owned());
                        }
                    }
                }
            }
        } else if f.field == 2 && f.wire == 2 && out.destination_address.is_none() {
            // first NavigationDestination { address=1 }
            for d in decode_fields(&f.bytes) {
                if d.field == 1 && d.wire == 2 {
                    out.destination_address = Some(String::from_utf8_lossy(&d.bytes).into_owned());
                }
            }
        }
    }
    out
}

// NavigationCurrentPosition { step_distance=1, destination_distances=2, current_road=3 }
fn decode_position(payload: &[u8]) -> NavigationPositionUpdate {
    let mut out = NavigationPositionUpdate::default();
    for f in decode_fields(payload) {
        if f.field == 1 && f.wire == 2 {
            // NavigationStepDistance { distance=1, time_to_step_seconds=2 }
            for s in decode_fields(&f.bytes) {
                if s.field == 1 && s.wire == 2 {
                    let d = decode_nav_distance(&s.bytes);
                    out.step_distance_meters = d.0;
                    out.step_distance_display = d.1;
                } else if s.field == 2 && s.wire == 0 {
                    out.time_to_step_seconds = Some(decode_varint_value(&s.bytes));
                }
            }
        } else if f.field == 2 && f.wire == 2 && out.destination_meters.is_none() {
            // first NavigationDestinationDistance { distance=1, eta=2, time_to_arrival=3 }
            for dd in decode_fields(&f.bytes) {
                if dd.field == 1 && dd.wire == 2 {
                    let d = decode_nav_distance(&dd.bytes);
                    out.destination_meters = d.0;
                    out.destination_display = d.1;
                    out.destination_units = d.2;
                } else if dd.field == 2 && dd.wire == 2 {
                    out.eta_text = Some(String::from_utf8_lossy(&dd.bytes).into_owned());
                } else if dd.field == 3 && dd.wire == 0 {
                    out.time_to_arrival_seconds = Some(decode_varint_value(&dd.bytes));
                }
            }
        } else if f.field == 3 && f.wire == 2 {
            // NavigationRoad { name=1 }
            for r in decode_fields(&f.bytes) {
                if r.field == 1 && r.wire == 2 {
                    out.current_road_name = Some(String::from_utf8_lossy(&r.bytes).into_owned());
                }
            }
        }
    }
    out
}

// NavigationDistance { meters=1, display_value=2, display_units=3 }
fn decode_nav_distance(b: &[u8]) -> (Option<u32>, Option<String>, Option<u32>) {
    let mut meters = None;
    let mut display = None;
    let mut units = None;
    for f in decode_fields(b) {
        if f.field == 1 && f.wire == 0 {
            meters = Some(decode_varint_value(&f.bytes));
        } else if f.field == 2 && f.wire == 2 {
            display = Some(String::from_utf8_lossy(&f.bytes).into_owned());
        } else if f.field == 3 && f.wire == 0 {
            units = Some(decode_varint_value(&f.bytes));
        }
    }
    (meters, display, units)
}

fn map_next_turn_enum(v: u32) -> NavigationTurnEvent {
    match v {
        1 => NavigationTurnEvent::Depart,
        2 => NavigationTurnEvent::NameChange,
        3 => NavigationTurnEvent::SlightTurn,
        4 => NavigationTurnEvent::Turn,
        5 => NavigationTurnEvent::SharpTurn,
        6 => NavigationTurnEvent::UTurn,
        7 => NavigationTurnEvent::OnRamp,
        8 => NavigationTurnEvent::OffRamp,
        9 => NavigationTurnEvent::Fork,
        10 => NavigationTurnEvent::Merge,
        11 => NavigationTurnEvent::RoundaboutEnter,
        12 => NavigationTurnEvent::RoundaboutExit,
        13 => NavigationTurnEvent::RoundaboutEnterAndExit,
        14 => NavigationTurnEvent::Straight,
        16 => NavigationTurnEvent::FerryBoat,
        17 => NavigationTurnEvent::FerryTrain,
        19 => NavigationTurnEvent::Destination,
        _ => NavigationTurnEvent::Unknown,
    }
}
