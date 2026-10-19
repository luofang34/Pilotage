//! Typed readings of the telemetry codes a client must interpret: the
//! estimator's validity and quality, the FC's arm report, its answer to
//! the last arm or disarm, the gimbal's yaw reference, and the vehicle's
//! magnetic variation.
//!
//! Every client reads these through this module, so no client decodes a
//! bit or a code a second time, and each code fails closed the same way
//! everywhere.

use pilotage_protocol::{
    CommandOutcome, GimbalAxisLocks, GimbalReference, GimbalYawFrame, HeadingReference,
    MagneticVariationSource, wire,
};

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

/// The magnetic variation a vehicle states for its own position.
#[derive(Debug, Clone, PartialEq)]
pub struct MagneticVariation {
    /// Where the value comes from.
    pub source: MagneticVariationSource,
    /// Declination: the angle from true north to magnetic north, radians,
    /// positive east. `None` when the vehicle applies no variation.
    pub declination_rad: Option<f32>,
    /// The geomagnetic model that produced the value, when stated.
    pub model_id: Option<String>,
    /// The model's epoch as a decimal year, when stated.
    pub model_epoch: Option<f64>,
}

impl MagneticVariation {
    /// Which north the vehicle's attitude yaw uses: magnetic when it
    /// applies no variation, true otherwise.
    #[must_use]
    pub const fn heading_reference(&self) -> HeadingReference {
        self.source.heading_reference()
    }
}

/// The variation an avionics publication states, or `None` when it states
/// none a client can use: no stamp, a source this build does not know, or
/// an applied declination that is not a number. A client then treats the
/// heading reference as unknown, never as true.
#[must_use]
pub fn magnetic_variation(avionics: &wire::AvionicsState) -> Option<MagneticVariation> {
    avionics.magnetic_variation_stamp.as_ref()?;
    let variation = avionics.magnetic_variation.as_ref()?;
    let source = MagneticVariationSource::from_wire(variation.source)?;
    let declination_rad = if source == MagneticVariationSource::None {
        None
    } else if variation.declination_rad.is_finite() {
        Some(variation.declination_rad)
    } else {
        return None;
    };
    Some(MagneticVariation {
        source,
        declination_rad,
        model_id: (!variation.model_id.is_empty()).then(|| variation.model_id.clone()),
        model_epoch: (variation.model_epoch != 0.0).then_some(variation.model_epoch),
    })
}

#[cfg(test)]
mod tests;
