//! Following an extra flight in its member's status (ADR-0052): the run
//! after the team's start is that flight. When the run ends, the original
//! line takes its verdict; a verdict still open goes on to follow the run's
//! later reports until the line's evidence settles.

use pilotage_protocol::wire;

use super::TeamLane;
use super::refly::ReflyState;

impl TeamLane {
    /// Follows a flying re-flight in its member's status; when it ends, the
    /// original line takes the re-flight's verdict.
    pub(super) fn refly_progress(&mut self, member: &str, status: &wire::MissionStatus) {
        if self.follow_settling_refly(member, status) {
            return;
        }
        let Some(refly) = self.reflies.iter_mut().find(|r| {
            r.member == member && matches!(r.state, ReflyState::Flying | ReflyState::Uncertain)
        }) else {
            return;
        };
        // The first run after the start is this flight, whatever state its
        // first report shows; a member that has no earlier run starts fresh.
        let later_run = match refly.prior_run.as_deref() {
            Some(prior) => status.run_id != prior,
            None => status.state == wire::MissionLaneState::Running as i32,
        };
        if refly.run_id.is_none() && !status.run_id.is_empty() && later_run {
            refly.run_id = Some(status.run_id.clone());
            if !refly.moved.is_empty() {
                // Moved lines are new work: the book follows their run.
                self.book.adopt(member, &status.run_id);
                self.book.record(status, &[]);
            }
        }
        if refly.run_id.as_deref() != Some(status.run_id.as_str()) {
            return;
        }
        let line_status = status
            .tasks
            .iter()
            .find(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32);
        let moved = !refly.moved.is_empty();
        let assignment = refly.assignment.clone();
        if status.state == wire::MissionLaneState::Complete as i32 {
            refly.state = ReflyState::Done;
            if let (Some(line_status), false) = (line_status, moved) {
                self.book
                    .apply_refly(&refly.owner, &refly.line, line_status, true);
            }
            if let Some(assignment) = assignment.as_deref() {
                self.moved_flight_ended(assignment, true);
            }
        } else if matches!(
            wire::MissionLaneState::try_from(status.state),
            Ok(wire::MissionLaneState::Failed | wire::MissionLaneState::Aborted)
        ) {
            refly.state = ReflyState::Failed;
            if let Some(assignment) = assignment.as_deref() {
                self.moved_flight_ended(assignment, false);
            }
        }
    }

    /// A later report of the run of a re-flight that ended while its
    /// verdict was open: the original line takes the evidence as it
    /// settles. Returns whether `status` was that run's.
    fn follow_settling_refly(&mut self, member: &str, status: &wire::MissionStatus) -> bool {
        let Some(refly) = self.reflies.iter().find(|r| {
            r.member == member
                && r.state == ReflyState::Done
                && r.moved.is_empty()
                && r.run_id.as_deref() == Some(status.run_id.as_str())
        }) else {
            return false;
        };
        let (owner, line) = (refly.owner.clone(), refly.line.clone());
        let flown = status
            .tasks
            .iter()
            .find(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32);
        if let Some(flown) = flown {
            self.book.apply_refly(&owner, &line, flown, false);
        }
        true
    }
}
