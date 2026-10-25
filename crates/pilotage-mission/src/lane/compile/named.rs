//! Objectives of kinds that the lane registers by name. Each kind is a
//! planner that declares its version and its typed parameters, as an
//! executor's operation does, and gives the steps of its plan. A new kind
//! is a planner module and its entry here: it needs no wire message, no
//! client command and no change to the lifecycle, the team or the display.

mod check;
mod record;
mod report;

use std::collections::BTreeMap;

use pilotage_mission_core::executor::{ValueSpec, admit_values};
use pilotage_protocol::wire;

use super::Compiled;
use super::operations::{self, Step};
use crate::lane::LaneConfig;

/// The steps of the plan of an objective with its admitted parameters, by
/// name, whose tasks belong to the proposal named last; why it cannot be
/// planned with them.
pub(super) type Planner =
    fn(&wire::MissionObjective, &BTreeMap<String, String>, &str) -> Result<Vec<Step>, String>;

/// One kind of objective the lane plans by name.
pub(super) struct NamedKind {
    /// The kind's name, as a named objective gives it.
    pub kind: &'static str,
    /// The version of its parameters.
    pub version: u32,
    /// Its parameters.
    pub parameters: &'static [ValueSpec],
    /// How it plans.
    pub plan: Planner,
}

/// The kinds the lane plans by name, each name once.
pub(super) const KINDS: &[NamedKind] = &[check::CHECK, record::RECORD, report::REPORT];

/// The named objective a report objective is.
pub(super) fn report(report: &wire::ReportObjective) -> wire::NamedObjective {
    wire::NamedObjective {
        kind: report::REPORT.kind.to_owned(),
        version: report::REPORT.version,
        parameters: report::parameters(report),
    }
}

/// The named objective a check objective is.
pub(super) fn check(check: &wire::CheckObjective) -> wire::NamedObjective {
    wire::NamedObjective {
        kind: check::CHECK.kind.to_owned(),
        version: check::CHECK.version,
        parameters: check::parameters(check),
    }
}

fn value(name: &str, value: &str) -> wire::MissionValue {
    wire::MissionValue {
        name: name.to_owned(),
        value: value.to_owned(),
    }
}

/// Compiles the named objective `named` of `objective` under `lane`'s
/// configuration.
pub(super) fn compile(
    objective: &wire::MissionObjective,
    named: &wire::NamedObjective,
    lane: &LaneConfig,
    proposal_id: String,
) -> Result<Compiled, String> {
    let kind = KINDS
        .iter()
        .find(|k| k.kind == named.kind)
        .ok_or_else(|| format!("this lane plans no objective of the kind {:?}", named.kind))?;
    if named.version != kind.version {
        return Err(format!(
            "the {} objective is version {}; this lane plans version {}",
            kind.kind, named.version, kind.version
        ));
    }
    let mut values = BTreeMap::new();
    for value in &named.parameters {
        if value.name.is_empty() {
            return Err("a parameter has no name".to_owned());
        }
        if values
            .insert(value.name.clone(), value.value.clone())
            .is_some()
        {
            return Err(format!("the parameter {} is given twice", value.name));
        }
    }
    admit_values(kind.kind, kind.parameters, &values).map_err(|refused| refused.to_string())?;
    let steps = (kind.plan)(objective, &values, &proposal_id)?;
    operations::compile(objective, lane, proposal_id, steps)
}

#[cfg(test)]
mod tests;
