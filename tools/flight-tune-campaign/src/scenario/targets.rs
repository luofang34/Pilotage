//! The frozen scoped bar the Alia 250 matrix is decided against.
//!
//! Every limit here is stated before any candidate exists and is keyed to one
//! exact scenario, so no limit can be loosened for a result that has already
//! been measured, and no limit written for one physical response can decide
//! another.
//!
//! The two hidden partitions carry different quantities. A promotion row is
//! the target the search drives toward; a final row is the compliance maximum
//! a delivered calibration must not exceed. The second is looser than the
//! first on purpose: a candidate that improves toward the target is what the
//! search is for, and the ceiling is what refuses one that never got there.

use std::path::Path;

use flight_tune::{
    MissionReference, PhysicalTarget, ResponseTargetScope, ResponseTargetTable,
    TargetAuthorityBand, TargetComparison,
};
use pilotage_trial::ControlFamily;

use super::{LoadedCell, LoadedMatrix, MatrixPartition, ScenarioMatrix};
use crate::error::{CampaignError, matrix};

/// One degree in radians.
const DEGREE: f64 = 0.017_453_292_519_943_295;

/// The overshoot each Alia roll and pitch step targets.
const ROLL_PITCH_OVERSHOOT_TARGET: f64 = 0.05;
/// The overshoot compliance maximum a roll or pitch step must not exceed.
const ROLL_PITCH_OVERSHOOT_MAXIMUM: f64 = 0.30;
/// The overshoot a yaw step must not exceed.
const YAW_OVERSHOOT_MAXIMUM: f64 = 0.10;
/// The time a roll or pitch step has to settle inside five percent.
const ROLL_PITCH_SETTLE_S: f64 = 1.0;
/// The time a yaw step has to settle inside five percent.
const YAW_SETTLE_S: f64 = 2.5;
/// The excursion past trim a return may reach.
const RETURN_PEAK_RAD: f64 = 0.5 * DEGREE;
/// The body-rate activity a trial may still carry in its final second.
const FINAL_BODY_RATE_RMS_RPS: f64 = 0.5 * DEGREE;
/// The share of the response window a collective command may spend
/// accelerating the wrong way.
const COLLECTIVE_DIRECTION_ERROR: f64 = 0.05;
/// The vertical acceleration a collective command has to produce.
const COLLECTIVE_PEAK_MPS2: f64 = 1.0;
/// The share of its requested target an operator input has to keep.
const MINIMUM_OPERATOR_AUTHORITY: f64 = 0.86;

/// The mission a matrix cell projects to.
///
/// A scoped limit is keyed to the executed document rather than to the
/// authored trial, so the bar and the run name one identity.
///
/// # Errors
///
/// Returns [`CampaignError`] when the projection or the reference fails.
pub fn matrix_mission(
    cell: &LoadedCell,
    max_samples: u32,
) -> Result<MissionReference, CampaignError> {
    let document = flight_tune::calibration_mission_document(&cell.scenario, 0, RECEIPT_TIMEOUT_NS)
        .map_err(|source| matrix(format!("{}: {source}", cell.scenario.id)))?;
    MissionReference::from_document(&document, max_samples)
        .map_err(|source| matrix(format!("{}: {source}", cell.scenario.id)))
}

/// How long one sample of an Alia matrix run may take to arrive.
const RECEIPT_TIMEOUT_NS: u64 = 400_000_000;
/// The sample ceiling one Alia matrix run may reach.
const MAX_SAMPLES: u32 = 1_200;

/// The complete frozen bar for the Alia matrix's two hidden partitions.
///
/// # Errors
///
/// Returns [`CampaignError`] when the corpus is not the declared matrix or a
/// scoped row is not valid.
pub fn alia250_matrix_response_targets(
    corpus: &Path,
) -> Result<ResponseTargetTable, CampaignError> {
    let loaded = LoadedMatrix::load_blocking(&ALIA250, corpus)?;
    let mut rows = Vec::new();
    for cell in loaded.cells() {
        let partition = cell.cell.partition;
        if partition == MatrixPartition::Training {
            continue;
        }
        let mission = matrix_mission(cell, MAX_SAMPLES)?;
        let scope = cell_scope(cell, &mission);
        rows.extend(scope.rows(limits(cell, partition)));
    }
    ResponseTargetTable::new(rows).map_err(|source| matrix(source.to_string()))
}

/// The declaration this bar is written against.
const ALIA250: ScenarioMatrix = super::ALIA250_MATRIX;

fn cell_scope(cell: &LoadedCell, mission: &MissionReference) -> ResponseTargetScope {
    let stimulus = cell.cell.stimulus;
    let family = match stimulus.family {
        ControlFamily::OperatorVelocity => flight_tune::ControlFamily::OperatorVelocity,
        ControlFamily::DirectAttitudeThrust => flight_tune::ControlFamily::DirectAttitudeThrust,
    };
    let channel = match stimulus.channel {
        pilotage_trial::ControlChannel::Roll => flight_tune::ControlChannel::Roll,
        pilotage_trial::ControlChannel::Pitch => flight_tune::ControlChannel::Pitch,
        pilotage_trial::ControlChannel::Vertical => flight_tune::ControlChannel::Vertical,
        pilotage_trial::ControlChannel::Yaw => flight_tune::ControlChannel::Yaw,
    };
    let target = stimulus.physical_target();
    ResponseTargetScope {
        mission_revision_id: mission.revision_id.clone(),
        mission_content_digest: mission.content_digest,
        control_family: family,
        control_channel: channel,
        physical_target: PhysicalTarget {
            unit: family.required_physics(channel).0,
            value: target,
        },
        envelope_digest: envelope_digest(cell),
        // Only an operator input resolves its own physical target, so only an
        // operator scope keeps authority over it.
        authority_band: matches!(family, flight_tune::ControlFamily::OperatorVelocity).then(|| {
            TargetAuthorityBand {
                minimum: target.abs() * MINIMUM_OPERATOR_AUTHORITY,
                maximum: target.abs(),
            }
        }),
    }
}

fn envelope_digest(cell: &LoadedCell) -> flight_tune::Digest {
    cell.scenario
        .phases
        .iter()
        .find_map(|phase| match &phase.action {
            pilotage_trial::PhaseAction::Stimulus { envelope, .. } => {
                envelope.canonical_digest().ok()
            }
            _ => None,
        })
        .map_or_else(
            || flight_tune::Digest::from_bytes([0; 32]),
            |digest| flight_tune::Digest::from_bytes(*digest.as_bytes()),
        )
}

/// The objective limits one cell's scope states in one partition.
fn limits(
    cell: &LoadedCell,
    partition: MatrixPartition,
) -> Vec<(&'static str, TargetComparison, f64)> {
    let promotion = partition == MatrixPartition::Promotion;
    let mut rows = vec![
        (
            "control.effort_rms",
            TargetComparison::AtMost,
            if promotion { 0.5 } else { 0.75 },
        ),
        (
            "control.saturation_fraction",
            TargetComparison::AtMost,
            if promotion { 0.02 } else { 0.05 },
        ),
    ];
    let stimulus = cell.cell.stimulus;
    match (stimulus.family, stimulus.channel) {
        (ControlFamily::DirectAttitudeThrust, pilotage_trial::ControlChannel::Vertical) => {
            rows.push((
                "collective.direction_error_fraction",
                TargetComparison::AtMost,
                COLLECTIVE_DIRECTION_ERROR,
            ));
            rows.push((
                "collective.peak_response_mps2",
                TargetComparison::AtLeast,
                COLLECTIVE_PEAK_MPS2,
            ));
        }
        (ControlFamily::DirectAttitudeThrust, channel) => {
            let yaw = channel == pilotage_trial::ControlChannel::Yaw;
            rows.push((
                "angular.overshoot_fraction",
                TargetComparison::AtMost,
                angular_overshoot(yaw, promotion),
            ));
            rows.push((
                "angular.settling_time_s",
                TargetComparison::AtMost,
                if yaw {
                    YAW_SETTLE_S
                } else {
                    ROLL_PITCH_SETTLE_S
                },
            ));
            rows.push((
                "angular_release.opposite_return_peak_rad",
                TargetComparison::AtMost,
                RETURN_PEAK_RAD,
            ));
            rows.push((
                "angular_release.final_body_rate_rms_rps",
                TargetComparison::AtMost,
                FINAL_BODY_RATE_RMS_RPS,
            ));
        }
        (ControlFamily::OperatorVelocity, _) => {
            rows.push((
                "response.overshoot_fraction",
                TargetComparison::AtMost,
                if promotion { 0.05 } else { 0.30 },
            ));
            rows.push((
                "response.settling_time_s",
                TargetComparison::AtMost,
                if promotion { 1.0 } else { 2.0 },
            ));
        }
    }
    rows
}

/// The overshoot bar one angular scope states.
///
/// A promotion row is the five percent target the search drives toward. A
/// final row is the compliance maximum: thirty percent for a roll or pitch
/// step and ten percent for a yaw step.
fn angular_overshoot(yaw: bool, promotion: bool) -> f64 {
    if promotion {
        ROLL_PITCH_OVERSHOOT_TARGET
    } else if yaw {
        YAW_OVERSHOOT_MAXIMUM
    } else {
        ROLL_PITCH_OVERSHOOT_MAXIMUM
    }
}
