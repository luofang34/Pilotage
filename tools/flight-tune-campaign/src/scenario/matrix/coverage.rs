//! Coverage counted from executable values, never from declarations.
//!
//! A matrix can report complete coverage from names alone: a cell labelled
//! "crosswind" whose artifact requests no wind covers nothing, and a checker
//! that read the label would not know. Every rule here compares the declared
//! factor against the exact value the decoded artifact carries, so a factor is
//! covered only by an artifact that would actually perturb the run.
//!
//! The artifact half is checkable here and now. The evidence half of the same
//! rule — that terminal evidence proves the requested value and the execution
//! counters — belongs to the run, and is recomputed where run evidence is
//! verified.

use std::collections::BTreeMap;

use pilotage_trial::{CommandLossPolicy, ConditionSet, DelayJitter, TurbulenceModel};

use super::LoadedCell;
use crate::error::{CampaignError, matrix};
use crate::scenario::{MatrixPartition, ScenarioMatrix, UncertaintyFactor};

/// The nominal value of a basis-point scale.
const BASIS_POINTS_NOMINAL: u16 = 10_000;

/// What one corpus proved about each declared factor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixReport {
    /// The number of cells the corpus carried.
    pub cells: usize,
    /// For each declared condition, the partitions whose artifacts carry its
    /// exact executable value.
    pub covered: BTreeMap<String, Vec<&'static str>>,
}

/// Requires every declared factor to be carried by an artifact of every
/// partition.
///
/// # Errors
///
/// Returns [`CampaignError`] when a partition has no artifact carrying a
/// declared factor's exact value.
pub(super) fn verify(
    declaration: &ScenarioMatrix,
    cells: &[LoadedCell],
) -> Result<MatrixReport, CampaignError> {
    let mut covered = BTreeMap::new();
    for condition in declaration.conditions {
        let mut partitions = Vec::new();
        for partition in MatrixPartition::ALL {
            let carried = cells.iter().any(|loaded| {
                loaded.cell.partition == partition
                    && loaded.cell.condition.id == condition.id
                    && carries(&loaded.condition, condition.factor)
            });
            if !carried {
                return Err(matrix(format!(
                    "no {} artifact carries the executable value of {}",
                    partition.as_str(),
                    condition.id
                )));
            }
            partitions.push(partition.as_str());
        }
        covered.insert(condition.id.to_owned(), partitions);
    }
    verify_calm_is_nominal(declaration, cells)?;
    Ok(MatrixReport {
        cells: cells.len(),
        covered,
    })
}

/// A calm artifact requests nothing, so a calm cell can never be counted as
/// covering an uncertainty factor.
fn verify_calm_is_nominal(
    declaration: &ScenarioMatrix,
    cells: &[LoadedCell],
) -> Result<(), CampaignError> {
    let calm = declaration.conditions[0].id;
    for loaded in cells.iter().filter(|entry| entry.cell.condition.id == calm) {
        for condition in declaration.uncertainty_conditions() {
            if carries(&loaded.condition, condition.factor) {
                return Err(matrix(format!(
                    "the calm artifact {} carries the {} perturbation",
                    loaded.condition.id, condition.id
                )));
            }
        }
    }
    Ok(())
}

/// Whether one artifact carries the exact executable value of one factor.
fn carries(condition: &ConditionSet, factor: UncertaintyFactor) -> bool {
    match factor {
        UncertaintyFactor::Calm => is_nominal(condition),
        UncertaintyFactor::SteadyWind {
            speed_mps,
            direction_deg,
        } => {
            condition.wind.steady.speed_mps == speed_mps
                && condition.wind.steady.direction_deg == direction_deg
        }
        UncertaintyFactor::Gust { speed_mps, hold_ns } => condition
            .wind
            .gusts
            .iter()
            .any(|gust| gust.speed_mps == speed_mps && gust.hold_ns == hold_ns),
        UncertaintyFactor::ActuatorAuthority { basis_points } => {
            condition.actuator.authority_scale_basis_points == basis_points
                && basis_points != BASIS_POINTS_NOMINAL
        }
        UncertaintyFactor::HoverTrim { basis_points } => {
            condition
                .controller_initialization
                .hover_thrust_force
                .scale_basis_points()
                == basis_points
                && basis_points != BASIS_POINTS_NOMINAL
        }
        UncertaintyFactor::SensorNoise { lanes } => condition.sensor.noise_lanes().len() == lanes,
        UncertaintyFactor::TimingJitter {
            maximum_delay_ns,
            interval_ns,
        } => matches!(
            condition.timing.update_jitter,
            DelayJitter::SampleAndHold {
                maximum_delay_ns: stated,
                interval_ns: stated_interval,
            } if stated == maximum_delay_ns && stated_interval == interval_ns
        ),
        UncertaintyFactor::AddedDelay { estimate_delay_ns } => {
            condition.timing.estimate_delay_ns == estimate_delay_ns && estimate_delay_ns != 0
        }
        UncertaintyFactor::CommandLoss {
            fraction_basis_points,
            decision_interval_samples,
        } => matches!(
            condition.actuator.command_loss,
            CommandLossPolicy::SeededZeroOrderHold {
                fraction_basis_points: stated,
                decision_interval_samples: stated_interval,
            } if stated == fraction_basis_points && stated_interval == decision_interval_samples
        ),
    }
}

/// Whether one artifact requests no perturbation of any kind.
fn is_nominal(condition: &ConditionSet) -> bool {
    condition.wind.steady.speed_mps == 0.0
        && condition.wind.gusts.is_empty()
        && matches!(condition.wind.turbulence, TurbulenceModel::None)
        && condition.timing.estimate_delay_ns == 0
        && matches!(condition.timing.update_jitter, DelayJitter::None)
        && condition.sensor.is_nominal()
        && condition.actuator.authority_scale_basis_points == BASIS_POINTS_NOMINAL
        && matches!(condition.actuator.command_loss, CommandLossPolicy::None {})
        && condition
            .controller_initialization
            .has_nominal_hover_thrust_force()
}
