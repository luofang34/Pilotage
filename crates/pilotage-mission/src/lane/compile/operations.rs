//! What every kind of objective that is a plan of operations shares: its
//! steps become operation tasks in order, each admitted only when the
//! lane's host carries it out with values of the declared types, and the
//! plan is the operations and how each step's result feeds the next.

use pilotage_mission_core::BoundAction;
use pilotage_mission_core::executor::{OperationDescriptor, ValueKind};
use pilotage_protocol::wire;

use super::{Compiled, seal};
use crate::lane::{LaneConfig, PlannedOperation, RunPlan};

/// One step of a plan: its task, its name and its operation.
pub(super) type Step = (String, &'static str, wire::MissionOperation);

/// The plan of `steps` for `objective` under `lane`'s configuration.
pub(super) fn compile(
    objective: &wire::MissionObjective,
    lane: &LaneConfig,
    proposal_id: String,
    steps: Vec<Step>,
) -> Result<Compiled, String> {
    let planned: Vec<(&str, &wire::MissionOperation)> = steps
        .iter()
        .map(|(id, _, step)| (id.as_str(), step))
        .collect();
    for (_, name, step) in &steps {
        admit(lane, &planned, step).map_err(|reason| format!("the {name} step: {reason}"))?;
    }
    let (proposal, config) = proposal(objective, lane, proposal_id, &steps);
    let (proposal, record) = seal(proposal);
    let plan = RunPlan::Operations(
        steps
            .into_iter()
            .map(|(task_id, _, operation)| PlannedOperation { task_id, operation })
            .collect(),
    );
    Ok(Compiled {
        proposal,
        record,
        plan,
        config,
        height_m: 0.0,
        min_usable_per_line: 1,
    })
}

/// The proposal of `steps` for `objective`, and the config its run takes.
fn proposal(
    objective: &wire::MissionObjective,
    lane: &LaneConfig,
    proposal_id: String,
    steps: &[(String, &'static str, wire::MissionOperation)],
) -> (wire::MissionProposal, crate::config::MissionConfig) {
    let tasks = steps
        .iter()
        .map(|(id, name, step)| wire::MissionTaskPlan {
            task_id: id.clone(),
            kind: wire::MissionTaskKind::Operation as i32,
            name: (*name).to_owned(),
            operation: Some(step.clone()),
            ..wire::MissionTaskPlan::default()
        })
        .collect::<Vec<_>>();
    let mut config = lane.template.clone();
    config.task_ids = tasks.iter().map(|t| t.task_id.clone()).collect();
    let proposal = wire::MissionProposal {
        proposal_id,
        objective_id: objective.objective_id.clone(),
        tasks,
        estimates: Some(wire::MissionEstimates::default()),
        assumptions: Some(wire::MissionAssumptions {
            execution_strategy: "host-executors".to_owned(),
            vehicle_name: lane.name.clone(),
            simulated: lane.simulated,
            ..wire::MissionAssumptions::default()
        }),
        objective: Some(objective.clone()),
        ..wire::MissionProposal::default()
    };
    (proposal, config)
}

pub(super) fn operation(
    descriptor: &OperationDescriptor,
    mut parameters: Vec<wire::MissionParameter>,
    met_when: Option<(&str, &str)>,
) -> wire::MissionOperation {
    parameters.sort_by(|a, b| a.name.cmp(&b.name));
    wire::MissionOperation {
        executor: descriptor.executor.to_owned(),
        operation: descriptor.operation.to_owned(),
        version: descriptor.version,
        parameters,
        met_when: met_when.map(|(name, value)| wire::MissionValue {
            name: name.to_owned(),
            value: value.to_owned(),
        }),
    }
}

pub(super) fn given(name: &str, value: &str) -> wire::MissionParameter {
    wire::MissionParameter {
        name: name.to_owned(),
        source: Some(wire::mission_parameter::Source::Value(value.to_owned())),
    }
}

/// The parameter `name` that takes the result `result` of the task `task`.
pub(super) fn result_of(name: &str, task: &str, result: &str) -> wire::MissionParameter {
    wire::MissionParameter {
        name: name.to_owned(),
        source: Some(wire::mission_parameter::Source::ResultOf(
            wire::MissionResultRef {
                task_id: task.to_owned(),
                result: result.to_owned(),
            },
        )),
    }
}

/// Whether the lane's host carries out `step`, one of `planned`: its
/// operation and version are registered, every value it is given is of its
/// declared type, and a result of an earlier task it takes is one that task
/// declares, of the same type. The run checks that result's value when it
/// comes.
fn admit(
    lane: &LaneConfig,
    planned: &[(&str, &wire::MissionOperation)],
    step: &wire::MissionOperation,
) -> Result<(), String> {
    let registered = |op: &wire::MissionOperation| {
        lane.executors
            .operations()
            .iter()
            .find(|d| d.executor == op.executor && d.operation == op.operation)
    };
    let descriptor = registered(step);
    let mut parameters = std::collections::BTreeMap::new();
    for parameter in &step.parameters {
        let value = match &parameter.source {
            Some(wire::mission_parameter::Source::Value(value)) => value.clone(),
            Some(wire::mission_parameter::Source::ResultOf(of)) => {
                let earlier = planned
                    .iter()
                    .take_while(|(_, op)| !std::ptr::eq(*op, step))
                    .find(|(id, _)| *id == of.task_id)
                    .ok_or_else(|| {
                        format!("{} takes a result of no earlier task", parameter.name)
                    })?;
                let declared = registered(earlier.1)
                    .and_then(|d| d.results.iter().find(|r| r.name == of.result))
                    .ok_or_else(|| {
                        format!(
                            "{} takes the result {}, which is not declared",
                            parameter.name, of.result
                        )
                    })?;
                let wanted = descriptor
                    .and_then(|d| d.parameters.iter().find(|p| p.name == parameter.name))
                    .map(|p| p.kind);
                if wanted.is_some_and(|kind| kind != declared.kind) {
                    return Err(format!(
                        "{} takes {}, of another type",
                        parameter.name, of.result
                    ));
                }
                sample(declared.kind)
            }
            None => return Err(format!("{} names no value", parameter.name)),
        };
        parameters.insert(parameter.name.clone(), value);
    }
    if let Some(wanted) = &step.met_when {
        let declared = descriptor.is_some_and(|d| d.results.iter().any(|r| r.name == wanted.name));
        if !declared {
            return Err(format!(
                "its judgment names {}, which is no result of it",
                wanted.name
            ));
        }
    }
    let action = BoundAction {
        executor: step.executor.clone(),
        operation: step.operation.clone(),
        version: step.version,
        parameters,
    };
    lane.executors
        .admit(&action)
        .map(|_| ())
        .map_err(|refused| refused.to_string())
}

/// A value of `kind`, standing for a result that comes at run time.
fn sample(kind: ValueKind) -> String {
    match kind {
        ValueKind::Text { .. } => String::new(),
        ValueKind::Integer { min, .. } => min.to_string(),
        ValueKind::Flag => "false".to_owned(),
    }
}
