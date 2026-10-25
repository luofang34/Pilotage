//! The check kind: read a value, hold it to a limit by a rule, and write a
//! report of the verdict. Each step is an operation of an executor, and a
//! rule decides; no person takes part.

use pilotage_mission_core::executor::{AT_MOST, READ_VALUE, WRITE_REPORT};
use pilotage_protocol::wire;

use super::operations::{self, Step, given, operation, result_of};
use super::{Compiled, task_id};
use crate::lane::LaneConfig;

/// Compiles the check `check` of `objective` under `lane`'s configuration.
pub(super) fn compile(
    objective: &wire::MissionObjective,
    check: &wire::CheckObjective,
    lane: &LaneConfig,
    proposal_id: String,
) -> Result<Compiled, String> {
    if check.source.is_empty() {
        return Err("the check objective names no source".to_owned());
    }
    pilotage_mission_core::executor::rule::limit(&check.limit)?;
    let steps = steps(objective, check, &proposal_id);
    operations::compile(objective, lane, proposal_id, steps)
}

/// The plan's steps: read the value, hold it to the limit, and write the
/// report of the verdict.
fn steps(
    objective: &wire::MissionObjective,
    check: &wire::CheckObjective,
    proposal_id: &str,
) -> Vec<Step> {
    let read = task_id(proposal_id, "read");
    let hold = task_id(proposal_id, "limit");
    let write = task_id(proposal_id, "write");
    vec![
        (
            read.clone(),
            "read",
            operation(&READ_VALUE, vec![given("source", &check.source)], None),
        ),
        (
            hold.clone(),
            "limit",
            operation(
                &AT_MOST,
                vec![
                    given("limit", &check.limit),
                    result_of("value", &read, "value"),
                ],
                None,
            ),
        ),
        (
            write,
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
    ]
}
