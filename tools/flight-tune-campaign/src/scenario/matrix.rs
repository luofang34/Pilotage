//! Checking one generated corpus against the matrix that declared it.
//!
//! Four things can be wrong with a corpus and each is refused separately: a
//! cell the declaration names and the corpus does not carry, a file the corpus
//! carries and the declaration does not name, two cells that would run the
//! same disturbance, and an artifact whose content is not the one its cell
//! asks for.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use pilotage_trial::{ConditionSet, ControlChannel, ControlFamily, Digest, PhaseAction, Scenario};

use super::projection::{
    condition_path, read_condition_blocking, read_scenario_blocking, scenario_path,
};
use super::{MatrixCell, ScenarioMatrix};
use crate::error::{CampaignError, matrix};

mod coverage;

pub use coverage::MatrixReport;

/// One declared cell and the two artifacts it resolved to.
#[derive(Debug, Clone)]
pub struct LoadedCell {
    /// The cell the declaration states.
    pub cell: MatrixCell,
    /// The scenario the corpus carries for it.
    pub scenario: Scenario,
    /// The condition the corpus carries for it.
    pub condition: ConditionSet,
}

/// A complete corpus, read back and checked against its declaration.
#[derive(Debug, Clone)]
pub struct LoadedMatrix {
    cells: Vec<LoadedCell>,
}

impl LoadedMatrix {
    /// Reads one corpus and checks it against the matrix that declared it.
    ///
    /// # Errors
    ///
    /// Returns [`CampaignError`] when a cell is missing, an artifact is not
    /// canonical, a file is an orphan, two cells share a seed, or an artifact
    /// does not carry the content its cell declares.
    pub fn load_blocking(declaration: &ScenarioMatrix, root: &Path) -> Result<Self, CampaignError> {
        let mut cells = Vec::with_capacity(declaration.expected_cell_count());
        let mut expected = BTreeSet::new();
        for cell in declaration.cells() {
            let scenario_file = scenario_path(root, &cell);
            let condition_file = condition_path(root, &cell);
            expected.insert(scenario_file.clone());
            expected.insert(condition_file.clone());
            let scenario = read_scenario_blocking(&scenario_file)?;
            let condition = read_condition_blocking(&condition_file)?;
            verify_cell(&cell, &scenario, &condition)?;
            cells.push(LoadedCell {
                cell,
                scenario,
                condition,
            });
        }
        verify_document_count(declaration, &cells)?;
        verify_no_orphans(root, &expected)?;
        verify_distinct_identities(&cells)?;
        Ok(Self { cells })
    }

    /// Every loaded cell, in declaration order.
    #[must_use]
    pub fn cells(&self) -> &[LoadedCell] {
        &self.cells
    }

    /// The coverage every declared factor reaches in this corpus.
    ///
    /// # Errors
    ///
    /// Returns [`CampaignError`] when a declared factor is not carried by any
    /// artifact of a partition that has to exercise it.
    pub fn coverage(&self, declaration: &ScenarioMatrix) -> Result<MatrixReport, CampaignError> {
        coverage::verify(declaration, &self.cells)
    }
}

/// The scenario and the condition state the cell the declaration names.
fn verify_cell(
    cell: &MatrixCell,
    scenario: &Scenario,
    condition: &ConditionSet,
) -> Result<(), CampaignError> {
    let stimulus = scenario_stimulus(cell, scenario)?;
    if stimulus.0 != cell.stimulus.family || stimulus.1 != cell.stimulus.channel {
        return Err(matrix(format!(
            "{} commands another control family",
            scenario.id
        )));
    }
    let declared = condition
        .canonical_digest()
        .map_err(|source| matrix(format!("{}: {source}", condition.id)))?;
    if scenario_condition_digest(scenario)? != declared {
        return Err(matrix(format!(
            "{} applies a condition identity its artifact does not produce",
            scenario.id
        )));
    }
    Ok(())
}

/// The one stimulus a matrix scenario commands.
fn scenario_stimulus(
    cell: &MatrixCell,
    scenario: &Scenario,
) -> Result<(ControlFamily, ControlChannel), CampaignError> {
    let mut found = None;
    for phase in &scenario.phases {
        if let PhaseAction::Stimulus {
            family,
            channel,
            envelope,
            ..
        } = &phase.action
        {
            if found.is_some() {
                return Err(matrix(format!("{} commands two stimuli", scenario.id)));
            }
            if envelope.id != cell.stimulus.envelope_id {
                return Err(matrix(format!(
                    "{} names envelope {}, not {}",
                    scenario.id, envelope.id, cell.stimulus.envelope_id
                )));
            }
            found = Some((*family, *channel));
        }
    }
    found.ok_or_else(|| matrix(format!("{} commands no stimulus", scenario.id)))
}

/// The condition identity a matrix scenario applies.
fn scenario_condition_digest(scenario: &Scenario) -> Result<Digest, CampaignError> {
    for phase in &scenario.phases {
        if let PhaseAction::ApplyConditions { condition_set } = &phase.action {
            return Ok(condition_set.digest);
        }
    }
    Err(matrix(format!("{} applies no condition", scenario.id)))
}

/// The corpus states exactly as many artifacts as the declaration produces.
fn verify_document_count(
    declaration: &ScenarioMatrix,
    cells: &[LoadedCell],
) -> Result<(), CampaignError> {
    let expected = declaration.expected_document_count();
    let stated = cells.len().saturating_mul(2);
    if stated != expected {
        return Err(matrix(format!(
            "the corpus states {stated} artifacts and the declaration produces {expected}"
        )));
    }
    Ok(())
}

/// No file exists that the declaration does not name.
///
/// An orphan is a scenario nobody schedules and nobody regenerates, so it can
/// drift from the contract without any check noticing.
fn verify_no_orphans(root: &Path, expected: &BTreeSet<PathBuf>) -> Result<(), CampaignError> {
    for directory in ["conditions", "scenarios"] {
        let target = root.join(directory);
        let entries = std::fs::read_dir(&target)
            .map_err(|source| matrix(format!("cannot read {}: {source}", target.display())))?;
        for entry in entries {
            let path = entry
                .map_err(|source| matrix(format!("cannot read {}: {source}", target.display())))?
                .path();
            if path.extension().is_some_and(|value| value == "json") && !expected.contains(&path) {
                return Err(matrix(format!(
                    "{} is an orphan the declaration does not name",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

/// No two cells share a scenario identity, a condition identity, or a seed.
///
/// Two cells on one seed run the same disturbance twice and cover one factor
/// once, and two cells on one identity are two schedules for one artifact.
fn verify_distinct_identities(cells: &[LoadedCell]) -> Result<(), CampaignError> {
    let mut scenarios = BTreeSet::new();
    let mut conditions = BTreeSet::new();
    let mut seeds: BTreeMap<u64, String> = BTreeMap::new();
    for loaded in cells {
        if !scenarios.insert(loaded.scenario.id.clone()) {
            return Err(matrix(format!(
                "scenario identity {} is declared twice",
                loaded.scenario.id
            )));
        }
        if !conditions.insert(loaded.condition.id.clone()) {
            return Err(matrix(format!(
                "condition identity {} is declared twice",
                loaded.condition.id
            )));
        }
        if let Some(other) = seeds.insert(loaded.condition.seed, loaded.scenario.id.clone()) {
            return Err(matrix(format!(
                "{other} and {} run the same seed",
                loaded.scenario.id
            )));
        }
    }
    Ok(())
}
