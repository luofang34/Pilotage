use std::collections::BTreeMap;

use flight_tune::{KinematicTruth, ObservedSignal, ScenarioFrame, VehicleLifecycleState};
use pilotage_xplane_trial::XPlaneTruthSample;
use thiserror::Error;

mod action_port;

pub use action_port::{XPlaneScenarioRuntime, XPlaneSimulatorAction, XPlaneSimulatorActionDriver};

/// Vehicle and condition values joined to one X-Plane truth sample.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VehicleFrameValues {
    /// The vehicle lifecycle state, when available.
    pub lifecycle: Option<VehicleLifecycleState>,
    /// The control-link validity, when available.
    pub link_valid: Option<bool>,
    /// The estimator validity, when available.
    pub estimator_valid: Option<bool>,
    /// Applied environmental values by canonical name.
    pub applied_conditions: BTreeMap<String, f64>,
    /// Canonical vehicle values that X-Plane truth cannot supply.
    pub canonical_signals: Vec<ObservedSignal>,
}

/// An X-Plane sample cannot produce a neutral scenario frame.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum XPlaneProjectionError {
    /// A sample time cannot be represented in nanoseconds.
    #[error("X-Plane {field} cannot be represented in nanoseconds")]
    InvalidTime {
        /// The invalid time field.
        field: &'static str,
    },
    /// The sample has an invalid kinematic value.
    #[error("X-Plane truth has an invalid kinematic value")]
    InvalidKinematics,
}

/// Stateful projection from verified X-Plane truth into neutral frames.
#[derive(Debug, Default)]
pub struct XPlaneFrameProjection {
    position_origin_ned_m: Option<[f64; 3]>,
}

impl XPlaneFrameProjection {
    /// Creates a projection with no latched run origin.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            position_origin_ned_m: None,
        }
    }

    /// Converts one X-Plane truth sample into one neutral scenario frame.
    ///
    /// Plant evidence, plugin evidence, process identity, and socket evidence
    /// stay outside the returned frame.
    ///
    /// # Errors
    ///
    /// Returns an error when time or kinematic values are invalid.
    pub fn project(
        &mut self,
        sample: &XPlaneTruthSample,
        vehicle: VehicleFrameValues,
    ) -> Result<ScenarioFrame, XPlaneProjectionError> {
        let simulator_time_ns = seconds_to_nanoseconds(sample.sim_time_s, "simulator time")?;
        let trial_time_ns = seconds_to_nanoseconds(sample.trial_time_s, "trial time")?;
        let position = sample.position_ned_m();
        let mut truth = KinematicTruth {
            position_ned_m: position,
            velocity_ned_mps: sample.velocity_ned_mps(),
            acceleration_ned_mps2: sample.acceleration_ned_mps2(),
            attitude_wxyz: sample.quaternion,
            body_rates_rps: sample.body_rates_rps,
        };
        if truth_values(&truth).any(|value| !value.is_finite()) {
            return Err(XPlaneProjectionError::InvalidKinematics);
        }
        let origin = *self.position_origin_ned_m.get_or_insert(position);
        truth.position_ned_m = subtract(position, origin);
        Ok(ScenarioFrame {
            source_sequence: sample.sequence,
            simulator_time_ns,
            trial_time_ns,
            lifecycle: vehicle.lifecycle,
            ground_contact: sample.on_ground,
            crashed: sample.crashed,
            link_valid: vehicle.link_valid,
            estimator_valid: vehicle.estimator_valid,
            truth,
            applied_conditions: vehicle.applied_conditions,
            canonical_signals: vehicle.canonical_signals,
        })
    }
}

fn seconds_to_nanoseconds(seconds: f64, field: &'static str) -> Result<u64, XPlaneProjectionError> {
    let nanoseconds = seconds * 1_000_000_000.0;
    if !seconds.is_finite()
        || seconds < 0.0
        || !nanoseconds.is_finite()
        || nanoseconds > u64::MAX as f64
    {
        return Err(XPlaneProjectionError::InvalidTime { field });
    }
    Ok(nanoseconds.round() as u64)
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn truth_values(truth: &KinematicTruth) -> impl Iterator<Item = f64> + '_ {
    truth
        .position_ned_m
        .iter()
        .chain(&truth.velocity_ned_mps)
        .chain(&truth.acceleration_ned_mps2)
        .chain(&truth.attitude_wxyz)
        .chain(&truth.body_rates_rps)
        .copied()
}

#[cfg(test)]
mod tests;
