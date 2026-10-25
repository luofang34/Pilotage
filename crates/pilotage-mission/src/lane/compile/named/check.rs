//! The check kind: read a value, hold it to a limit by a rule, and write a
//! report of the verdict. A rule decides; no person takes part.

use std::collections::BTreeMap;

use pilotage_mission_core::executor::{AT_MOST, READ_VALUE, ValueKind, ValueSpec, WRITE_REPORT};
use pilotage_protocol::wire;

use super::NamedKind;
use crate::lane::compile::operations::{Step, given, operation, result_of};
use crate::lane::compile::task_id;

pub(super) const CHECK: NamedKind = NamedKind {
    kind: "check",
    version: 1,
    parameters: &[
        ValueSpec {
            name: "limit",
            kind: ValueKind::Text { max_bytes: 64 },
            required: true,
        },
        ValueSpec {
            name: "source",
            kind: ValueKind::Text { max_bytes: 128 },
            required: true,
        },
    ],
    plan,
};

/// The check objective's parameters, as the lane plans it by name.
pub(super) fn parameters(check: &wire::CheckObjective) -> Vec<wire::MissionValue> {
    vec![
        super::value("limit", &check.limit),
        super::value("source", &check.source),
    ]
}

fn plan(
    objective: &wire::MissionObjective,
    values: &BTreeMap<String, String>,
    proposal_id: &str,
) -> Result<Vec<Step>, String> {
    let given_value = |name: &str| values.get(name).cloned().unwrap_or_default();
    let source = given_value("source");
    if source.is_empty() {
        return Err("the check objective names no source".to_owned());
    }
    let limit = given_value("limit");
    pilotage_mission_core::executor::rule::limit(&limit)?;
    let read = task_id(proposal_id, "read");
    let hold = task_id(proposal_id, "limit");
    Ok(vec![
        (
            read.clone(),
            "read",
            operation(&READ_VALUE, vec![given("source", &source)], None),
        ),
        (
            hold.clone(),
            "limit",
            operation(
                &AT_MOST,
                vec![given("limit", &limit), result_of("value", &read, "value")],
                None,
            ),
        ),
        (
            task_id(proposal_id, "write"),
            "report",
            operation(
                &WRITE_REPORT,
                vec![
                    given("title", &objective.title),
                    result_of("value", &hold, "verdict"),
                ],
                None,
            ),
        ),
    ])
}
