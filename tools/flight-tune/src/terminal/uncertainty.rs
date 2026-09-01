//! What one run declared it would execute under uncertainty.
//!
//! A run intent binds a condition identity. That alone does not say the
//! controller received the requested uncertainty: a backend can accept a
//! condition, start, and apply nothing. The declaration here states the
//! executable content of one condition in the units the executor applies,
//! so a later reader can derive every seeded decision again and compare it
//! with what the run reports.
//!
//! Nothing in this module reads an executed value. It states the declared
//! side of the relation only. SIM / NOT FOR FLIGHT.

use pilotage_trial::{BackendCapability, CommandLossPolicy, ConditionSet, SensorReferenceLane};
use serde::{Deserialize, Serialize};

use super::digest::domain_digest;
use super::invalid_terminal;
use crate::{Digest, TuneError};

pub mod derivation;
mod launch;
mod ledger;
mod receipt;
mod sample;
mod stream;

pub use launch::{ExecutedLaunchIdentity, executed_run_seed};
pub use ledger::{
    ExecutedActuatorCounts, ExecutedBypassCounts, ExecutedSensorLaneCounts,
    ExecutedUncertaintyLedger,
};
pub use receipt::ExecutedUncertaintyReceipt;
pub use sample::{
    EXECUTED_ACTUATOR_LANE_COUNT, ExecutedActuatorApplication, ExecutedBypassReason,
    ExecutedConstraintFlags, ExecutedEligibility, ExecutedHoverInitialization, ExecutedSample,
    ExecutedSendEvidence, ExecutedSensorApplication,
};
pub use stream::{ExecutedStream, ExecutedStreamSummary};

/// The supported executed-uncertainty evidence schema.
pub const EXECUTED_UNCERTAINTY_SCHEMA_VERSION: u16 = 1;

/// The number of flight-controller sensor lanes the contract names.
pub const EXECUTED_SENSOR_LANE_COUNT: usize = 12;

/// The basis-point value that requests no scaling.
pub const NOMINAL_BASIS_POINTS: u16 = 10_000;

const DECLARATION_DOMAIN: &[u8] = b"pilotage.flight-tune.executed-uncertainty-declaration.v1\0";

/// One declared sensor noise lane, in the unit the executor applies.
///
/// The artifact declares gauss and hectopascals; the flight controller reads
/// microtesla and pascal. The amplitude is stated here in the executor unit
/// so a reader never has to know which side of the conversion it holds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredSensorLane {
    /// The one-byte lane tag in the noise preimage.
    pub lane_tag: u8,
    /// The executor-unit peak amplitude as its exact binary form.
    pub peak_amplitude_bits: u32,
    /// The number of samples one drawn offset is held for.
    pub update_interval_samples: u32,
}

/// The declared deterministic command hold.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredCommandHold {
    /// The held fraction of one complete decision interval.
    pub fraction_basis_points: u16,
    /// The number of eligible commands in one decision interval.
    pub decision_interval_samples: u32,
}

/// Everything one run declared it would execute.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedUncertaintyDeclaration {
    /// Declaration schema version.
    pub schema_version: u16,
    /// The canonical condition identity every decision is seeded from.
    pub condition_digest: Digest,
    /// The exact bytes of the artifact the executor loaded.
    pub artifact_digest: Digest,
    /// The seed that separates one execution of this condition from another.
    pub run_seed: u64,
    /// The declared sensor lanes, in ascending lane-tag order.
    pub sensor_lanes: Vec<DeclaredSensorLane>,
    /// The declared eligible-command authority scale.
    pub authority_scale_basis_points: u16,
    /// The declared command hold, when the condition requests one.
    pub command_hold: Option<DeclaredCommandHold>,
    /// The declared hover feed-forward scale.
    pub hover_scale_basis_points: u16,
    /// The capabilities the executor must supply, in ascending name order.
    pub required_capabilities: Vec<BackendCapability>,
}

impl ExecutedUncertaintyDeclaration {
    /// States what one verified condition and run seed will execute.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the condition is not valid, when it names
    /// a lane twice, or when an identity is absent.
    pub fn from_condition(
        condition: &ConditionSet,
        artifact_digest: Digest,
        run_seed: u64,
    ) -> Result<Self, TuneError> {
        let condition_digest = condition
            .canonical_digest()
            .map_err(|source| invalid_terminal(format!("condition identity refused: {source}")))?;
        let mut required_capabilities = condition.required_capabilities();
        required_capabilities.sort_unstable_by_key(|capability| capability.as_str());
        let declaration = Self {
            schema_version: EXECUTED_UNCERTAINTY_SCHEMA_VERSION,
            condition_digest,
            artifact_digest,
            run_seed,
            sensor_lanes: declared_lanes(condition),
            authority_scale_basis_points: condition.actuator.authority_scale_basis_points,
            command_hold: declared_hold(&condition.actuator.command_loss),
            hover_scale_basis_points: hover_scale(condition),
            required_capabilities,
        };
        declaration.validate()?;
        Ok(declaration)
    }

    /// Rejects a declaration that cannot seed a deterministic decision.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the schema differs, when an identity is
    /// absent, when a lane tag repeats or is out of range, or when a
    /// declared interval is zero.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema_version != EXECUTED_UNCERTAINTY_SCHEMA_VERSION {
            return Err(invalid_terminal("the executed uncertainty schema changed"));
        }
        if self.condition_digest.is_zero() || self.artifact_digest.is_zero() {
            return Err(invalid_terminal(
                "an executed uncertainty declaration has no condition identity",
            ));
        }
        self.validate_lanes()?;
        if let Some(hold) = self.command_hold
            && (hold.decision_interval_samples == 0 || hold.fraction_basis_points == 0)
        {
            return Err(invalid_terminal("a declared command hold holds nothing"));
        }
        self.validate_capabilities()
    }

    /// Returns the identity a run receipt binds this declaration by.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the declaration cannot be encoded.
    pub fn declaration_digest(&self) -> Result<Digest, TuneError> {
        domain_digest(DECLARATION_DOMAIN, self, "executed uncertainty declaration")
    }

    /// Returns the declared lane for one lane tag.
    #[must_use]
    pub fn lane(&self, lane_tag: u8) -> Option<DeclaredSensorLane> {
        self.sensor_lanes
            .iter()
            .copied()
            .find(|lane| lane.lane_tag == lane_tag)
    }

    /// Reports whether this declaration requests no executable uncertainty.
    #[must_use]
    pub fn is_nominal(&self) -> bool {
        self.sensor_lanes.is_empty()
            && self.command_hold.is_none()
            && self.authority_scale_basis_points == NOMINAL_BASIS_POINTS
            && self.hover_scale_basis_points == NOMINAL_BASIS_POINTS
    }

    fn validate_lanes(&self) -> Result<(), TuneError> {
        if self.sensor_lanes.len() > EXECUTED_SENSOR_LANE_COUNT {
            return Err(invalid_terminal(
                "a declaration names more lanes than the contract has",
            ));
        }
        let mut previous: Option<u8> = None;
        for lane in &self.sensor_lanes {
            if usize::from(lane.lane_tag) >= EXECUTED_SENSOR_LANE_COUNT
                || lane.update_interval_samples == 0
                || !f32::from_bits(lane.peak_amplitude_bits).is_finite()
            {
                return Err(invalid_terminal("a declared sensor lane is not executable"));
            }
            if previous.is_some_and(|prior| prior >= lane.lane_tag) {
                return Err(invalid_terminal(
                    "the declared sensor lanes are not in lane order",
                ));
            }
            previous = Some(lane.lane_tag);
        }
        Ok(())
    }

    fn validate_capabilities(&self) -> Result<(), TuneError> {
        let mut previous: Option<&'static str> = None;
        for capability in &self.required_capabilities {
            let name = capability.as_str();
            if previous.is_some_and(|prior| prior >= name) {
                return Err(invalid_terminal(
                    "the required capabilities are not in name order",
                ));
            }
            previous = Some(name);
        }
        let expected = self.derived_capabilities();
        if self.required_capabilities != expected {
            return Err(invalid_terminal(
                "the required capabilities do not follow from the declared factors",
            ));
        }
        Ok(())
    }

    fn derived_capabilities(&self) -> Vec<BackendCapability> {
        let mut derived = Vec::new();
        if self.authority_scale_basis_points != NOMINAL_BASIS_POINTS {
            derived.push(BackendCapability::ActuatorAuthority);
        }
        if self.command_hold.is_some() {
            derived.push(BackendCapability::CommandHold);
        }
        if self.hover_scale_basis_points != NOMINAL_BASIS_POINTS {
            derived.push(BackendCapability::HoverTrimUncertainty);
        }
        if !self.sensor_lanes.is_empty() {
            derived.push(BackendCapability::SensorPerturbation);
        }
        derived
    }
}

fn declared_lanes(condition: &ConditionSet) -> Vec<DeclaredSensorLane> {
    let mut lanes = condition
        .sensor
        .noise_lanes()
        .iter()
        .map(|request| {
            let (lane, amplitude, interval) = request.reference_values();
            DeclaredSensorLane {
                lane_tag: lane_tag(lane),
                peak_amplitude_bits: amplitude.to_bits(),
                update_interval_samples: interval,
            }
        })
        .collect::<Vec<_>>();
    lanes.sort_unstable_by_key(|lane| lane.lane_tag);
    lanes
}

fn declared_hold(policy: &CommandLossPolicy) -> Option<DeclaredCommandHold> {
    match policy {
        CommandLossPolicy::None {} => None,
        CommandLossPolicy::SeededZeroOrderHold {
            fraction_basis_points,
            decision_interval_samples,
        } => Some(DeclaredCommandHold {
            fraction_basis_points: *fraction_basis_points,
            decision_interval_samples: *decision_interval_samples,
        }),
    }
}

fn hover_scale(condition: &ConditionSet) -> u16 {
    condition
        .controller_initialization
        .hover_thrust_force
        .scale_basis_points()
}

/// Returns the one-byte preimage tag for one sensor lane.
#[must_use]
pub const fn lane_tag(lane: SensorReferenceLane) -> u8 {
    lane as u8
}

#[cfg(test)]
mod tests;
