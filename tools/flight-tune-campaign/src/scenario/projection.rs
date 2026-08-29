//! Where one declared cell's artifacts live, and how they are read back.
//!
//! The path is derived from the cell rather than discovered, so a checker
//! looking for a cell asks for the exact file the declaration names and finds
//! it or does not. A checker that globbed the directory would count whatever
//! it found as the matrix.

use std::path::{Path, PathBuf};

use pilotage_trial::{ConditionSet, Scenario};

use super::MatrixCell;
use crate::error::{CampaignError, matrix};

/// The condition artifact one cell applies.
#[must_use]
pub fn condition_path(root: &Path, cell: &MatrixCell) -> PathBuf {
    root.join("conditions").join(format!(
        "{}.{}.{}.json",
        cell.partition.as_str(),
        cell.condition.id,
        cell.stimulus.id
    ))
}

/// The scenario artifact one cell commands.
#[must_use]
pub fn scenario_path(root: &Path, cell: &MatrixCell) -> PathBuf {
    root.join("scenarios").join(format!(
        "{}.{}.{}.json",
        cell.partition.as_str(),
        cell.stimulus.id,
        cell.condition.id
    ))
}

/// Reads one artifact and requires its bytes to be its own canonical form.
///
/// The condition identity is the digest of the exact artifact bytes, so an
/// artifact that decodes to the right values through different bytes has a
/// different identity from the one the scenario names. Requiring the file to
/// be canonical is what keeps a hand-edited artifact from carrying one
/// meaning and two identities.
pub(super) fn read_condition_blocking(path: &Path) -> Result<ConditionSet, CampaignError> {
    let bytes = read_bytes(path)?;
    let condition = ConditionSet::from_json(&bytes)
        .map_err(|source| matrix(format!("{}: {source}", path.display())))?;
    let canonical = condition
        .to_canonical_json()
        .map_err(|source| matrix(format!("{}: {source}", path.display())))?;
    if canonical != bytes {
        return Err(matrix(format!(
            "{} is not its own canonical encoding",
            path.display()
        )));
    }
    Ok(condition)
}

/// Reads one scenario artifact and requires the same canonical property.
pub(super) fn read_scenario_blocking(path: &Path) -> Result<Scenario, CampaignError> {
    let bytes = read_bytes(path)?;
    let scenario = Scenario::from_json(&bytes)
        .map_err(|source| matrix(format!("{}: {source}", path.display())))?;
    let canonical = scenario
        .to_canonical_json()
        .map_err(|source| matrix(format!("{}: {source}", path.display())))?;
    if canonical != bytes {
        return Err(matrix(format!(
            "{} is not its own canonical encoding",
            path.display()
        )));
    }
    Ok(scenario)
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, CampaignError> {
    std::fs::read(path)
        .map_err(|source| matrix(format!("cannot read {}: {source}", path.display())))
}
