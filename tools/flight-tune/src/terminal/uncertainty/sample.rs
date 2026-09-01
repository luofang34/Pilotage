//! One sample of executed-uncertainty evidence.
//!
//! The executor states, for every simulation sample, what it read, what it
//! changed, what it commanded, and what it sent. A sample carries the raw
//! and effective values themselves rather than a summary, so a reader can
//! derive the requested decision again and compare it with the applied one.
//!
//! The types here state shape only. The relation between a declaration and
//! these values is derived elsewhere, so a sample can never answer for
//! itself.

use serde::{Deserialize, Serialize};

use super::super::invalid_terminal;
use super::EXECUTED_SENSOR_LANE_COUNT;
use crate::{Digest, TuneError};

/// Sensor values before and after one deterministic perturbation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedSensorApplication {
    /// The identity of the values read before any change.
    pub raw_digest: Digest,
    /// The identity of the values the controller received.
    pub effective_digest: Digest,
    /// The lanes this sample carried, in stable lane order.
    pub presence_mask: u16,
    /// The lanes whose exact value moved.
    pub changed_mask: u16,
    /// The held-offset bucket for each configured and present lane.
    pub update_buckets: [Option<u64>; EXECUTED_SENSOR_LANE_COUNT],
    /// The exact values read, in stable lane order.
    pub raw_value_bits: [Option<u32>; EXECUTED_SENSOR_LANE_COUNT],
    /// The exact values the controller received, in stable lane order.
    pub effective_value_bits: [Option<u32>; EXECUTED_SENSOR_LANE_COUNT],
}

/// Why one sample carried no actuator perturbation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutedBypassReason {
    /// The controller supplied no actuator answer.
    MissingAnswer,
    /// The answer named a different number of lanes.
    InvalidActuatorCount,
    /// A backup path produced the command.
    Backup,
    /// A direct command reached the actuator without external provenance.
    Direct,
    /// A failsafe produced the command.
    Failsafe,
    /// The controller raised a fallback lane.
    FallbackMask,
    /// The armed state changed on this sample.
    ArmTransition,
    /// The vehicle was not armed.
    Disarmed,
    /// An emergency termination produced the command.
    EmergencyTermination,
}

impl ExecutedBypassReason {
    /// Gets the stable reason name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingAnswer => "missing-answer",
            Self::InvalidActuatorCount => "invalid-actuator-count",
            Self::Backup => "backup",
            Self::Direct => "direct",
            Self::Failsafe => "failsafe",
            Self::FallbackMask => "fallback-mask",
            Self::ArmTransition => "arm-transition",
            Self::Disarmed => "disarmed",
            Self::EmergencyTermination => "emergency-termination",
        }
    }

    /// Reports whether this reason is a safety path.
    ///
    /// A safety command must reach the actuator as the controller wrote it,
    /// so it bypasses the hold and the authority scale together.
    #[must_use]
    pub const fn is_safety(self) -> bool {
        matches!(
            self,
            Self::Failsafe | Self::EmergencyTermination | Self::Disarmed | Self::ArmTransition
        )
    }
}

/// Whether one sample was eligible for an actuator perturbation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "reason", rename_all = "kebab-case")]
pub enum ExecutedEligibility {
    /// The sample was eligible.
    Eligible,
    /// The sample bypassed every perturbation policy.
    Bypass(ExecutedBypassReason),
}

/// The number of actuator lanes the contract states.
pub const EXECUTED_ACTUATOR_LANE_COUNT: usize = 16;

/// Actuator values before the plant constraints and transforms.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedActuatorApplication {
    /// The exact force-domain values the controller requested.
    pub requested_lane_bits: [u32; EXECUTED_ACTUATOR_LANE_COUNT],
    /// The exact values after the authority scale.
    pub authority_scaled_lane_bits: [u32; EXECUTED_ACTUATOR_LANE_COUNT],
    /// The exact values the plant boundary received.
    pub effective_lane_bits: [u32; EXECUTED_ACTUATOR_LANE_COUNT],
    /// The number of active actuator lanes.
    pub lane_count: u8,
    /// Whether this sample was eligible for a perturbation.
    pub eligibility: ExecutedEligibility,
    /// Whether this sample only primed the accepted-command history.
    pub prime: bool,
    /// The interval epoch, which a bypass advances.
    pub interval_epoch: Option<u64>,
    /// The interval index inside the current epoch.
    pub interval_index: Option<u64>,
    /// The zero-based position inside the current interval.
    pub interval_position: Option<u32>,
    /// The identity of the current interval.
    pub interval_identity: Option<Digest>,
    /// Whether the seeded schedule selected a hold at this position.
    pub selected_hold: bool,
    /// Whether the executor applied a hold at this position.
    pub applied_hold: bool,
    /// Whether this sample completed the interval.
    pub interval_complete: bool,
}

/// The plant constraints one sample reported.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedConstraintFlags {
    /// A requested injection did not reach the plant unchanged.
    pub injection_clamp: bool,
    /// The answer named a different number of lanes.
    pub invalid_actuator_count: bool,
    /// The controller supplied no actuator answer.
    pub missing_actuator_answer: bool,
    /// A collective rate limit changed a lane.
    pub collective_rate: bool,
    /// A mean ceiling changed a lane.
    pub mean_ceiling: bool,
    /// A single-lane ceiling changed a lane.
    pub lane_ceiling: bool,
    /// A ground constraint changed a lane.
    pub ground_squeeze: bool,
    /// The trace path itself failed on this sample.
    pub trace_failure: bool,
}

impl ExecutedConstraintFlags {
    /// Reports whether any constraint changed a commanded value.
    #[must_use]
    pub const fn clamped(self) -> bool {
        self.injection_clamp
            || self.collective_rate
            || self.mean_ceiling
            || self.lane_ceiling
            || self.ground_squeeze
    }
}

/// The controller hover initialization one sample repeats.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedHoverInitialization {
    /// The preset force-domain baseline.
    pub baseline_force_bits: u32,
    /// The force-domain value the controller was built with.
    pub effective_force_bits: u32,
    /// The applied hover-force scale.
    pub scale_basis_points: u16,
    /// Whether no online component can write the hover force.
    pub estimator_disabled: bool,
    /// The resolved kernel identity that carries this value.
    pub kernel_config_hash: u64,
}

/// The one actuator send this sample attempted.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedSendEvidence {
    /// Whether the executor attempted the send.
    pub attempted: bool,
    /// Whether the send completed.
    pub succeeded: bool,
    /// The sample time the send echoed.
    pub echoed_timestamp_us: u64,
    /// Whether the send answered the sensor sample in lockstep.
    pub lockstep: bool,
}

/// One complete sample of executed-uncertainty evidence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedSample {
    /// The gap-free sequence the trace publisher assigned.
    pub sequence: u64,
    /// The global sample sequence every seeded decision uses.
    pub global_sample_sequence: u64,
    /// The simulator sample time.
    pub simulator_timestamp_us: u64,
    /// The sensor evidence, when this sample carried sensor values.
    pub sensor: Option<ExecutedSensorApplication>,
    /// The actuator evidence, when this sample commanded the actuator.
    pub actuator: Option<ExecutedActuatorApplication>,
    /// The plant constraints this sample reported.
    pub constraints: ExecutedConstraintFlags,
    /// The repeated controller hover initialization.
    pub hover: ExecutedHoverInitialization,
    /// The one send this sample attempted.
    pub send: ExecutedSendEvidence,
    /// The kernel armed state after this sample.
    pub armed: bool,
}

impl ExecutedSample {
    /// Rejects a sample whose own values contradict each other.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a mask names an absent lane, when a
    /// present lane carries no value, when the actuator lane count is out
    /// of range, or when the interval state is only partly stated.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.constraints.trace_failure {
            return Err(invalid_terminal("a sample reports its own trace failure"));
        }
        if let Some(sensor) = self.sensor {
            sensor.validate()?;
        }
        if let Some(actuator) = self.actuator {
            actuator.validate()?;
        }
        Ok(())
    }
}

impl ExecutedSensorApplication {
    /// Rejects sensor evidence whose masks and values disagree.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a changed lane is absent, when a present
    /// lane carries no value, or when an absent lane carries one.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.raw_digest.is_zero() || self.effective_digest.is_zero() {
            return Err(invalid_terminal("a sensor sample has no value identity"));
        }
        if self.changed_mask & !self.presence_mask != 0 {
            return Err(invalid_terminal("a sensor sample changed an absent lane"));
        }
        for lane in 0..EXECUTED_SENSOR_LANE_COUNT {
            let present = self.presence_mask & (1 << lane) != 0;
            if present != self.raw_value_bits[lane].is_some()
                || present != self.effective_value_bits[lane].is_some()
            {
                return Err(invalid_terminal(
                    "a sensor lane presence bit does not match its value",
                ));
            }
            if !present && self.update_buckets[lane].is_some() {
                return Err(invalid_terminal("an absent sensor lane holds an offset"));
            }
        }
        Ok(())
    }
}

impl ExecutedActuatorApplication {
    /// Rejects actuator evidence whose own state is not complete.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the lane count is out of range, when the
    /// interval state is only partly stated, or when a bypassed sample
    /// still claims a schedule position.
    pub fn validate(&self) -> Result<(), TuneError> {
        if usize::from(self.lane_count) > EXECUTED_ACTUATOR_LANE_COUNT || self.lane_count == 0 {
            return Err(invalid_terminal("an actuator sample states no active lane"));
        }
        let stated = [
            self.interval_epoch.is_some(),
            self.interval_index.is_some(),
            self.interval_position.is_some(),
            self.interval_identity.is_some(),
        ];
        if stated.iter().any(|part| *part) && !stated.iter().all(|part| *part) {
            return Err(invalid_terminal("an interval identity is only part stated"));
        }
        if self.interval_identity.is_some_and(Digest::is_zero) {
            return Err(invalid_terminal("an interval identity is absent"));
        }
        if self.eligibility != ExecutedEligibility::Eligible
            && (self.selected_hold || self.applied_hold || stated[0])
        {
            return Err(invalid_terminal(
                "a bypassed sample claims a hold schedule position",
            ));
        }
        if self.prime && (self.selected_hold || self.applied_hold) {
            return Err(invalid_terminal("a priming sample claims a hold"));
        }
        Ok(())
    }
}
