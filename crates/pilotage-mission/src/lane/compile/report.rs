//! The report kind: read a value, ask a person to confirm it, and write a
//! report of it. Each step is one operation of a host executor, so the
//! plan is only the operations, their order and how each step's result
//! feeds the next; the run, its answers and its judgment are the lane's.

use pilotage_mission_core::executor::{READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT};
use pilotage_protocol::wire;

use super::operations::{self, given, operation, result_of};
use super::{Compiled, task_id};
use crate::lane::LaneConfig;

/// Compiles the report `report` of `objective` under `lane`'s configuration.
pub(super) fn compile(
    objective: &wire::MissionObjective,
    report: &wire::ReportObjective,
    lane: &LaneConfig,
    proposal_id: String,
) -> Result<Compiled, String> {
    if report.source.is_empty() {
        return Err("the report objective names no source".to_owned());
    }
    let steps = steps(objective, report, &proposal_id);
    operations::compile(objective, lane, proposal_id, steps.to_vec())
}

/// The plan's steps, each with its task and its name: read the value,
/// ask the person to confirm it, and write the report of it.
fn steps(
    objective: &wire::MissionObjective,
    report: &wire::ReportObjective,
    proposal_id: &str,
) -> [(String, &'static str, wire::MissionOperation); 3] {
    let read = task_id(proposal_id, "read");
    let confirm = task_id(proposal_id, "confirm");
    let write = task_id(proposal_id, "write");
    [
        (
            read.clone(),
            "read",
            operation(&READ_VALUE, vec![given("source", &report.source)], None),
        ),
        (
            confirm.clone(),
            "confirm",
            operation(
                &WAIT_FOR_PERSON,
                // The person confirms the value the read returned.
                vec![
                    given("question", &report.question),
                    result_of("subject", &read, "value"),
                ],
                // A person who declines does not confirm the value.
                Some(("confirmed", "true")),
            ),
        ),
        (
            write,
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
    ]
}
