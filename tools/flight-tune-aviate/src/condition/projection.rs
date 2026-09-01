//! Projecting one executor frame onto the evidence this launcher answers for.
//!
//! The executor states more about a sample than the uncertainty relation
//! needs. The projection keeps exactly the values the relation is derived
//! from, so a field this repository cannot check never enters the counts it
//! publishes.

use flight_tune::{
    Digest, ExecutedActuatorApplication, ExecutedBypassReason, ExecutedConstraintFlags,
    ExecutedEligibility, ExecutedHoverInitialization, ExecutedSample, ExecutedSendEvidence,
    ExecutedSensorApplication,
};

use super::error::AviateConditionError;
use super::protocol::{
    TuningActuatorApplication, TuningActuatorBypassReason, TuningActuatorEligibility,
    TuningControlObservation, TuningFrameType, TuningSensorApplication,
};

/// Projects one observation onto the sample the relation is derived from.
///
/// # Errors
///
/// Returns [`AviateConditionError`] when the frame is not an observation,
/// when it speaks another protocol version, when it reports its own trace
/// failure, or when it names a kernel other than the handshake one.
pub fn sample(
    observation: &TuningControlObservation,
    schema_version: u16,
    estimator_disabled: bool,
    kernel_config_hash: u64,
) -> Result<ExecutedSample, AviateConditionError> {
    if observation.frame_type != TuningFrameType::AviateControlObservation {
        return Err(AviateConditionError::protocol(
            "a trace frame is not an observation",
        ));
    }
    if observation.schema_version != schema_version {
        return Err(AviateConditionError::protocol(
            "an observation speaks another trace protocol version",
        ));
    }
    if observation.constraint_flags.tuning_trace_failure {
        return Err(AviateConditionError::protocol(
            "an observation reports its own trace failure",
        ));
    }
    if observation.hover_initialization.kernel_config_hash != kernel_config_hash {
        return Err(AviateConditionError::identity(
            "a sample names another kernel than the one the handshake stated",
        ));
    }
    Ok(ExecutedSample {
        sequence: observation.sequence,
        global_sample_sequence: observation.global_sample_sequence,
        simulator_timestamp_us: observation.simulator_timestamp_us,
        sensor: observation.sensor_application.map(sensor),
        actuator: observation.actuator_application.map(actuator),
        constraints: constraints(observation),
        hover: ExecutedHoverInitialization {
            baseline_force_bits: observation.hover_initialization.baseline_force_bits,
            effective_force_bits: observation.hover_initialization.effective_force_bits,
            scale_basis_points: observation.hover_initialization.scale_basis_points,
            estimator_disabled,
            kernel_config_hash: observation.hover_initialization.kernel_config_hash,
        },
        send: ExecutedSendEvidence {
            attempted: observation.send.reply_attempted,
            succeeded: observation.send.reply_succeeded,
            echoed_timestamp_us: observation.send.echoed_timestamp_us,
            lockstep: observation.send.lockstep,
        },
        armed: observation.armed,
    })
}

fn sensor(application: TuningSensorApplication) -> ExecutedSensorApplication {
    ExecutedSensorApplication {
        raw_digest: Digest::from_bytes(application.raw_digest),
        effective_digest: Digest::from_bytes(application.effective_digest),
        presence_mask: application.presence_mask,
        changed_mask: application.changed_mask,
        update_buckets: application.update_buckets,
        raw_value_bits: application.raw_value_bits,
        effective_value_bits: application.effective_value_bits,
    }
}

fn actuator(application: TuningActuatorApplication) -> ExecutedActuatorApplication {
    ExecutedActuatorApplication {
        requested_lane_bits: application.requested_lane_bits,
        authority_scaled_lane_bits: application.authority_scaled_lane_bits,
        effective_lane_bits: application.effective_lane_bits,
        lane_count: application.lane_count,
        eligibility: eligibility(application.eligibility),
        prime: application.prime,
        interval_epoch: application.interval_epoch,
        interval_index: application.interval_index,
        interval_position: application.interval_position,
        interval_identity: application.interval_identity.map(Digest::from_bytes),
        selected_hold: application.selected_hold,
        applied_hold: application.applied_hold,
        interval_complete: application.interval_complete,
    }
}

const fn eligibility(value: TuningActuatorEligibility) -> ExecutedEligibility {
    match value {
        TuningActuatorEligibility::Eligible => ExecutedEligibility::Eligible,
        TuningActuatorEligibility::Bypass(reason) => ExecutedEligibility::Bypass(match reason {
            TuningActuatorBypassReason::MissingAnswer => ExecutedBypassReason::MissingAnswer,
            TuningActuatorBypassReason::InvalidActuatorCount => {
                ExecutedBypassReason::InvalidActuatorCount
            }
            TuningActuatorBypassReason::Backup => ExecutedBypassReason::Backup,
            TuningActuatorBypassReason::Direct => ExecutedBypassReason::Direct,
            TuningActuatorBypassReason::Failsafe => ExecutedBypassReason::Failsafe,
            TuningActuatorBypassReason::FallbackMask => ExecutedBypassReason::FallbackMask,
            TuningActuatorBypassReason::ArmTransition => ExecutedBypassReason::ArmTransition,
            TuningActuatorBypassReason::Disarmed => ExecutedBypassReason::Disarmed,
            TuningActuatorBypassReason::EmergencyTermination => {
                ExecutedBypassReason::EmergencyTermination
            }
        }),
    }
}

const fn constraints(observation: &TuningControlObservation) -> ExecutedConstraintFlags {
    let flags = observation.constraint_flags;
    ExecutedConstraintFlags {
        injection_clamp: flags.injection_clamp,
        invalid_actuator_count: flags.invalid_actuator_count,
        missing_actuator_answer: flags.missing_actuator_answer,
        collective_rate: flags.collective_rate,
        mean_ceiling: flags.mean_ceiling,
        lane_ceiling: flags.lane_ceiling,
        ground_squeeze: flags.ground_squeeze,
        trace_failure: flags.tuning_trace_failure,
    }
}
