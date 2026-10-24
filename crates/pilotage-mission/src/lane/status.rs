//! The lane's status as it stands: the plan, the run's tasks, the successor and what
//! the run does now.

use pilotage_protocol::wire;

use super::run::{self, RunTracker};
use super::{Lane, RunExecution};

impl Lane {
    /// The status as it stands, with the sequence of the last change.
    #[must_use]
    pub fn status(&self) -> wire::MissionStatus {
        let proposal = self.proposal.as_ref().map(|compiled| &compiled.proposal);
        let survey = proposal
            .and_then(|p| p.objective.as_ref())
            .and_then(|o| o.kind.as_ref())
            .map(|wire::mission_objective::Kind::Survey(survey)| survey);
        wire::MissionStatus {
            lane: self.config.name.clone(),
            state: self.state as i32,
            objective_id: proposal.map(|p| p.objective_id.clone()).unwrap_or_default(),
            proposal_digest: proposal.map(|p| p.digest.clone()).unwrap_or_default(),
            plan_digest: proposal
                .and_then(|p| p.assumptions.as_ref())
                .map(|a| a.plan_digest.clone())
                .unwrap_or_default(),
            run_id: self
                .run
                .as_ref()
                .map(RunTracker::run_id)
                .unwrap_or_default(),
            tasks: self
                .past
                .iter()
                .flat_map(RunTracker::task_statuses)
                .chain(match (self.run.as_ref(), proposal) {
                    (Some(run), _) => run.task_statuses(),
                    (None, Some(p)) => run::planned_statuses(&p.tasks),
                    (None, None) => Vec::new(),
                })
                .collect(),
            active_task: self
                .run
                .as_ref()
                .and_then(RunTracker::active_task)
                .unwrap_or_default(),
            recovery_policy: self.config.recovery_policy.clone(),
            estimates: proposal.and_then(|p| p.estimates),
            simulated: self.config.simulated,
            detail: self.detail.clone(),
            members: Vec::new(),
            sequence: self.sequence,
            area: survey.map(|s| s.area.clone()).unwrap_or_default(),
            height_m: survey.map_or(0.0, |s| s.height_m),
            successor: self.successor_status(),
            // A team sets it on its member entries.
            member_link: wire::MemberLink::Unspecified as i32,
            execution: self
                .execution
                .filter(|_| self.is_flying())
                .map(RunExecution::wire),
            holds: self.hold_reasons(),
            released: self.released.clone(),
            handovers: Vec::new(),
            predecessor_run_id: self
                .run
                .as_ref()
                .and_then(|run| run.predecessor.clone())
                .unwrap_or_default(),
        }
    }
}
