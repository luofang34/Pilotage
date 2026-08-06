//! The state-group vocabulary: stable identities for every group a
//! source can supply (ADR-0029 extensible state groups).
//!
//! A group id is a wire tag, a descriptor requirement, and a status key
//! all at once, so the enum is the single registration point: adding a
//! group adds one variant, and the exhaustive matches over it (wire
//! codec, minimum length, status reporting, withholding) each become a
//! compile error until the new group is handled there.
//!
//! # Id registry (append-only)
//!
//! Assigned ids never change meaning and are never reused. Reserved
//! ranges, recorded here so an allocation is a doc edit before it is a
//! variant:
//!
//! | id | group |
//! |----|-------|
//! | 0x00 | never assigned (guards zeroed memory) |
//! | 0x01–0x0B | the variants below |
//! | 0x0C | monitor text (machine-monitoring readout; planned) |
//! | 0x0D | engine (planned) |
//! | 0x0E | traffic (planned) |
//! | 0x0F | projection view (synthetic vision; planned) |
//! | 0x10 | terrain bands (planned) |
//! | 0x11–0xDF | future standard groups |
//! | 0xE0–0xEF | experimentation; never in committed fixtures |
//! | 0xF0–0xFF | never assigned |

use crate::aircraft::AircraftState;
use crate::signal::SignalStatus;

/// Stable identity of one state group.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GroupId {
    /// Attitude quaternion and body rates.
    Attitude = 0x01,
    /// NED position and velocity.
    Kinematics = 0x02,
    /// Air data: indicated airspeed and applied altimeter setting.
    Air = 0x03,
    /// Lateral/vertical navigation guidance, including waypoint idents.
    Nav = 0x04,
    /// Wind estimate.
    Wind = 0x05,
    /// Pilot selections and bugs.
    Selections = 0x06,
    /// Source trust: quality, validity flags, snapshot coherence and
    /// generation.
    Trust = 0x07,
    /// Datum-qualified altitude declaration.
    Altitude = 0x08,
    /// Independent, reference-typed heading sample.
    Heading = 0x09,
    /// Magnetic-variation sample.
    Variation = 0x0A,
    /// Typed turn and slip/skid estimates.
    Dynamics = 0x0B,
}

impl GroupId {
    /// Number of defined groups.
    pub const COUNT: usize = 11;

    /// Every defined group in ascending id order — the canonical wire
    /// order and the index order of [`GroupStatuses`].
    pub const ALL: [GroupId; Self::COUNT] = [
        GroupId::Attitude,
        GroupId::Kinematics,
        GroupId::Air,
        GroupId::Nav,
        GroupId::Wind,
        GroupId::Selections,
        GroupId::Trust,
        GroupId::Altitude,
        GroupId::Heading,
        GroupId::Variation,
        GroupId::Dynamics,
    ];

    /// The wire tag.
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// The group for a wire tag; `None` for ids this build cannot place
    /// (the codec counts and skips them).
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(GroupId::Attitude),
            0x02 => Some(GroupId::Kinematics),
            0x03 => Some(GroupId::Air),
            0x04 => Some(GroupId::Nav),
            0x05 => Some(GroupId::Wind),
            0x06 => Some(GroupId::Selections),
            0x07 => Some(GroupId::Trust),
            0x08 => Some(GroupId::Altitude),
            0x09 => Some(GroupId::Heading),
            0x0A => Some(GroupId::Variation),
            0x0B => Some(GroupId::Dynamics),
            _ => None,
        }
    }

    /// Position in [`Self::ALL`], for dense per-group tables.
    pub const fn index(self) -> usize {
        (self as u8 as usize) - 1
    }
}

/// Per-group status, keyed by [`GroupId`] — the generic surface a
/// registry or harness asks instead of a method per group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GroupStatuses([SignalStatus; GroupId::COUNT]);

impl GroupStatuses {
    /// The status of one group.
    pub fn status(&self, id: GroupId) -> SignalStatus {
        self.0[id.index()]
    }

    /// Sets the status of one group (resolution-internal).
    pub(crate) fn set(&mut self, id: GroupId, status: SignalStatus) {
        self.0[id.index()] = status;
    }
}

/// `state` with one group withheld, exactly as if the source had never
/// fed it: stamped groups lose data and age, declared groups return to
/// their fail-closed defaults, and the validity flags covering the group
/// are cleared. The admission harness drives panels with this to prove
/// a withheld required group renders `Missing`, never a value.
pub fn withhold_group(state: &AircraftState, group: GroupId) -> AircraftState {
    let mut out = *state;
    match group {
        GroupId::Attitude => {
            out.attitude = Default::default();
            out.valid.attitude = false;
            out.valid.rates = false;
        }
        GroupId::Kinematics => {
            out.kinematics = Default::default();
            out.valid.position = false;
            out.valid.velocity = false;
        }
        GroupId::Air => out.air = Default::default(),
        GroupId::Nav => out.nav = Default::default(),
        GroupId::Wind => out.wind = Default::default(),
        GroupId::Selections => out.selections = Default::default(),
        GroupId::Trust => {
            out.quality = Default::default();
            out.valid = Default::default();
            out.snapshot = Default::default();
        }
        GroupId::Altitude => out.altitude = Default::default(),
        GroupId::Heading => {
            out.heading = Default::default();
            out.valid.heading = false;
        }
        GroupId::Variation => {
            out.variation = Default::default();
            out.valid.variation = false;
        }
        GroupId::Dynamics => {
            out.dynamics = Default::default();
            out.valid.turn = false;
            out.valid.slip = false;
        }
    }
    out
}

#[cfg(test)]
mod tests;
