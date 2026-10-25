//! The record kind: read a value and write a report of it. No person and
//! no rule take part.

use std::collections::BTreeMap;

use pilotage_mission_core::executor::{READ_VALUE, ValueKind, ValueSpec, WRITE_REPORT};
use pilotage_protocol::wire;

use super::NamedKind;
use crate::lane::compile::operations::{Step, given, operation, result_of};
use crate::lane::compile::task_id;

pub(super) const RECORD: NamedKind = NamedKind {
    kind: "record",
    version: 1,
    parameters: &[ValueSpec {
        name: "source",
        kind: ValueKind::Text { max_bytes: 128 },
        required: true,
    }],
    plan,
};

fn plan(
    objective: &wire::MissionObjective,
    values: &BTreeMap<String, String>,
    proposal_id: &str,
) -> Result<Vec<Step>, String> {
    let source = values.get("source").cloned().unwrap_or_default();
    if source.is_empty() {
        return Err("the record objective names no source".to_owned());
    }
    let read = task_id(proposal_id, "read");
    Ok(vec![
        (
            read.clone(),
            "read",
            operation(&READ_VALUE, vec![given("source", &source)], None),
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
