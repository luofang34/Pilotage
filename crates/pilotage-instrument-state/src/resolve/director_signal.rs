//! Resolution of the flight-director command group (#261).

use crate::aircraft::AircraftState;
use crate::signal::{FreshnessPolicy, SignalStatus};
use crate::validate::StateIntegrity;

use super::{Trust, group_freshness};

/// The director as the panel consumes it: one status for the whole
/// command (the group fold), the commanded attitude, and what
/// commands it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedDirector {
    /// Commanded pitch, radians; meaningful only when `status` shows.
    pub pitch_cmd_rad: f32,
    /// Commanded roll, radians; meaningful only when `status` shows.
    pub roll_cmd_rad: f32,
    /// What produces the command.
    pub mode: crate::director::FdMode,
    /// Whether the director is commanding.
    pub engagement: crate::director::FdEngagement,
    /// The group fold: freshness, trust, and integrity together.
    pub status: SignalStatus,
}

impl Default for ResolvedDirector {
    fn default() -> Self {
        Self {
            pitch_cmd_rad: 0.0,
            roll_cmd_rad: 0.0,
            mode: crate::director::FdMode::Unknown,
            engagement: crate::director::FdEngagement::Unknown,
            status: SignalStatus::Missing,
        }
    }
}

pub(super) fn director_resolved(
    state: &AircraftState,
    policy: &FreshnessPolicy,
    trust: &Trust,
    integrity: &StateIntegrity,
) -> ResolvedDirector {
    let has = state.director.data.is_some();
    let fresh = group_freshness(policy, has, state.director.age_ms);
    let status = trust.fold(has, fresh, integrity.director, true);
    let sample = state.director.data.unwrap_or_default();
    ResolvedDirector {
        pitch_cmd_rad: sample.pitch_cmd_rad,
        roll_cmd_rad: sample.roll_cmd_rad,
        mode: sample.mode,
        engagement: sample.engagement,
        status,
    }
}
