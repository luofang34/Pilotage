use std::collections::BTreeMap;

use pilotage_mission_core::{
    MissionObservation, ObservedSignal, QuaternionComponent, SignalSelector, VectorComponent,
    VehicleLifecycleState, VehicleObservation,
};

use super::ScenarioRuntimeError;

/// Simulator-neutral kinematic truth in north-east-down and body frames.
#[derive(Debug, Clone, PartialEq)]
pub struct KinematicTruth {
    /// Position in the local north-east-down frame, in meters.
    pub position_ned_m: [f64; 3],
    /// Velocity in the local north-east-down frame, in meters per second.
    pub velocity_ned_mps: [f64; 3],
    /// Kinematic acceleration in the local north-east-down frame.
    pub acceleration_ned_mps2: [f64; 3],
    /// Body attitude quaternion in scalar-first order.
    pub attitude_wxyz: [f64; 4],
    /// Body angular rates in radians per second.
    pub body_rates_rps: [f64; 3],
}

/// One simulator-neutral frame for a calibration mission tick.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioFrame {
    /// The source sample sequence.
    pub source_sequence: u64,
    /// The absolute simulator time in nanoseconds.
    pub simulator_time_ns: u64,
    /// The elapsed trial time in nanoseconds.
    pub trial_time_ns: u64,
    /// The vehicle lifecycle state, when available.
    pub lifecycle: Option<VehicleLifecycleState>,
    /// The ground-contact state, when available.
    pub ground_contact: Option<bool>,
    /// The crash state, when available.
    pub crashed: Option<bool>,
    /// The control-link validity, when available.
    pub link_valid: Option<bool>,
    /// The estimator validity, when available.
    pub estimator_valid: Option<bool>,
    /// The complete kinematic truth for this tick.
    pub truth: KinematicTruth,
    /// Applied environmental values by canonical name.
    pub applied_conditions: BTreeMap<String, f64>,
    /// Canonical values that the simulator truth projection does not supply.
    pub canonical_signals: Vec<ObservedSignal>,
}

impl ScenarioFrame {
    pub(super) fn mission_observation(&self) -> Result<MissionObservation, ScenarioRuntimeError> {
        self.validate()?;
        let mut signals = truth_signals(&self.truth);
        signals.extend(
            self.applied_conditions
                .iter()
                .map(|(name, value)| ObservedSignal {
                    selector: SignalSelector::ConditionValue { name: name.clone() },
                    value: *value,
                }),
        );
        signals.extend(self.canonical_signals.iter().cloned());
        if signals.iter().enumerate().any(|(index, signal)| {
            signals[..index]
                .iter()
                .any(|prior| prior.selector == signal.selector)
        }) {
            return Err(ScenarioRuntimeError::InvalidFrame {
                detail: "the frame repeats a canonical signal selector".to_owned(),
            });
        }
        Ok(MissionObservation {
            navigation: Default::default(),
            vehicle: VehicleObservation {
                lifecycle: self.lifecycle,
                ground_contact: self.ground_contact,
                crashed: self.crashed,
                link_valid: self.link_valid,
                estimator_valid: self.estimator_valid,
            },
            signals,
        })
    }

    fn validate(&self) -> Result<(), ScenarioRuntimeError> {
        if self.trial_time_ns > self.simulator_time_ns {
            return Err(ScenarioRuntimeError::InvalidFrame {
                detail: "the trial time is after the simulator time".to_owned(),
            });
        }
        if all_values(&self.truth).any(|value| !value.is_finite())
            || self.applied_conditions.iter().any(|(name, value)| {
                name.trim().is_empty() || name.len() > 256 || !value.is_finite()
            })
            || self
                .canonical_signals
                .iter()
                .any(|signal| !signal.value.is_finite())
        {
            return Err(ScenarioRuntimeError::InvalidFrame {
                detail: "the frame has an invalid name or numeric value".to_owned(),
            });
        }
        Ok(())
    }
}

fn all_values(truth: &KinematicTruth) -> impl Iterator<Item = f64> + '_ {
    truth
        .position_ned_m
        .iter()
        .chain(&truth.velocity_ned_mps)
        .chain(&truth.acceleration_ned_mps2)
        .chain(&truth.attitude_wxyz)
        .chain(&truth.body_rates_rps)
        .copied()
}

fn truth_signals(truth: &KinematicTruth) -> Vec<ObservedSignal> {
    let mut signals = Vec::with_capacity(16);
    add_vector(&mut signals, &truth.position_ned_m, |component| {
        SignalSelector::TruthPosition { component }
    });
    add_vector(&mut signals, &truth.velocity_ned_mps, |component| {
        SignalSelector::TruthVelocity { component }
    });
    add_vector(&mut signals, &truth.acceleration_ned_mps2, |component| {
        SignalSelector::TruthAcceleration { component }
    });
    for (component, value) in [
        (QuaternionComponent::W, truth.attitude_wxyz[0]),
        (QuaternionComponent::X, truth.attitude_wxyz[1]),
        (QuaternionComponent::Y, truth.attitude_wxyz[2]),
        (QuaternionComponent::Z, truth.attitude_wxyz[3]),
    ] {
        signals.push(ObservedSignal {
            selector: SignalSelector::TruthAttitude { component },
            value,
        });
    }
    add_vector(&mut signals, &truth.body_rates_rps, |component| {
        SignalSelector::TruthBodyRate { component }
    });
    signals
}

fn add_vector(
    signals: &mut Vec<ObservedSignal>,
    values: &[f64; 3],
    selector: impl Fn(VectorComponent) -> SignalSelector,
) {
    for (component, value) in [
        (VectorComponent::X, values[0]),
        (VectorComponent::Y, values[1]),
        (VectorComponent::Z, values[2]),
    ] {
        signals.push(ObservedSignal {
            selector: selector(component),
            value,
        });
    }
}
