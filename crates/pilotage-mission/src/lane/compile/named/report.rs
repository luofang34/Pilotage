//! The report kind: read a value, ask a person to confirm it, and write a
//! report of it. A person who declines does not confirm the value, and the
//! report is not written.

use std::collections::BTreeMap;

use pilotage_mission_core::executor::{
    READ_VALUE, ValueKind, ValueSpec, WAIT_FOR_PERSON, WRITE_REPORT,
};
use pilotage_protocol::wire;

use super::NamedKind;
use crate::lane::compile::operations::{Step, given, operation, result_of};
use crate::lane::compile::task_id;

pub(super) const REPORT: NamedKind = NamedKind {
    kind: "report",
    version: 1,
    parameters: &[
        ValueSpec {
            name: "question",
            kind: ValueKind::Text { max_bytes: 512 },
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

/// The report objective's parameters, as the lane plans it by name.
pub(super) fn parameters(report: &wire::ReportObjective) -> Vec<wire::MissionValue> {
    vec![
        super::value("question", &report.question),
        super::value("source", &report.source),
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
        return Err("the report objective names no source".to_owned());
    }
    let read = task_id(proposal_id, "read");
    Ok(vec![
        (
            read.clone(),
            "read",
            operation(&READ_VALUE, vec![given("source", &source)], None),
        ),
        (
            task_id(proposal_id, "confirm"),
            "confirm",
            operation(
                &WAIT_FOR_PERSON,
                // The person confirms the value the read returned.
                vec![
                    given("question", &given_value("question")),
                    result_of("subject", &read, "value"),
                ],
                // A person who declines does not confirm the value.
                Some(("confirmed", "true")),
            ),
        ),
        (
            task_id(proposal_id, "write"),
            "report",
            operation(
                &WRITE_REPORT,
                vec![
                    given("title", &objective.title),
                    result_of("value", &read, "value"),
                ],
                None,
            ),
        ),
    ])
}
