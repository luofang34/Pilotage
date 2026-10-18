//! Typed readings of the telemetry codes a client must interpret: the
//! estimator's validity and quality, the FC's arm report, its answer to
//! the last arm or disarm, and the gimbal's yaw reference.
//!
//! Every client reads these through this module, so no client decodes a
//! bit or a code a second time, and each code fails closed the same way
//! everywhere.

use pilotage_protocol::{CommandOutcome, GimbalAxisLocks, GimbalReference, GimbalYawFrame, wire};

/// Which estimate groups the estimator authorizes for use
/// (`AvionicsState.valid_flags`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EstimatorValidity {
    /// The attitude is valid (bit 0).
    pub attitude: bool,
    /// The body rates are valid (bit 1).
    pub rates: bool,
    /// The position is valid (bit 2).
    pub position: bool,
    /// The velocity is valid (bit 3).
    pub velocity: bool,
}

impl EstimatorValidity {
    /// The groups a `valid_flags` value authorizes. Bits above bit 3
    /// authorize nothing.
    #[must_use]
    pub const fn from_flags(flags: u32) -> Self {
        Self {
            attitude: flags & 1 != 0,
            rates: flags & 2 != 0,
            position: flags & 4 != 0,
            velocity: flags & 8 != 0,
        }
    }
}

/// The estimator's own rating of its estimate (`AvionicsState.quality`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstimateQuality {
    /// The estimate is good.
    Good,
    /// The estimate is usable with a caution.
    Degraded,
    /// The estimate must not be used.
    Unusable,
}

impl EstimateQuality {
    /// The rating a `quality` code carries; `None` for a code this build
    /// does not know, which a consumer treats as no rating.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Good),
            1 => Some(Self::Degraded),
            2 => Some(Self::Unusable),
            _ => None,
        }
    }
}

/// The FC's own arm report (`FcState.arm_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmState {
    /// The FC reports disarmed.
    Disarmed,
    /// The FC reports armed.
    Armed,
}

impl ArmState {
    /// The report an `arm_state` code carries; `None` for 0 (unknown) and
    /// for a code this build does not know.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(Self::Disarmed),
            2 => Some(Self::Armed),
            _ => None,
        }
    }
}

/// The FC's answer to the most recent commanded arm or disarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandVerdict {
    /// True when the answered command was an arm, false for a disarm.
    pub arm: bool,
    /// The typed answer; `None` when the host sent no outcome this build
    /// knows. It is never an acceptance.
    pub outcome: Option<CommandOutcome>,
}

/// The verdict an FC-state report carries, or `None` when it reports no
/// commanded arm or disarm.
#[must_use]
pub fn command_verdict(state: &wire::FcState) -> Option<CommandVerdict> {
    let arm = match state.last_command_kind {
        1 => true,
        2 => false,
        _ => return None,
    };
    Some(CommandVerdict {
        arm,
        outcome: CommandOutcome::from_wire(state.last_command_outcome),
    })
}

/// How to read a gimbal orientation report, or `None` when the report
/// names no yaw frame: its yaw then has no frame to show it against.
#[must_use]
pub fn gimbal_reference(gimbal: &wire::GimbalAttitude) -> Option<GimbalReference> {
    Some(GimbalReference {
        yaw_frame: GimbalYawFrame::from_wire(gimbal.yaw_frame)?,
        locks: GimbalAxisLocks {
            roll: gimbal.roll_locked,
            pitch: gimbal.pitch_locked,
            yaw: gimbal.yaw_locked,
        },
    })
}

#[cfg(test)]
mod tests;
