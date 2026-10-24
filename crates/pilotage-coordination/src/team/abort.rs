//! What a team abort ends (ADR-0052). The abort goes to every member, and
//! the team ends what it was still setting in motion: a re-flight or moved
//! lines not yet started are cancelled, and the receipts they wait for take
//! no next step. A flight whose start was sent may have started, so it
//! keeps its member and its uncertainty until the member reports. Committed
//! work that its receiver had not started is released and unassigned. A
//! release fact that arrives later is recorded, but it starts nothing.

use super::TeamLane;
use super::refly::ReflyState;
use pilotage_protocol::wire::MissionHandoverState as State;

impl TeamLane {
    /// Ends the work an abort of the team leaves pending.
    pub(super) fn end_pending_work(&mut self) {
        self.pending.clear();
        self.renewal = None;
        for refly in &mut self.reflies {
            refly.state = match refly.state {
                ReflyState::Queued | ReflyState::Proposing | ReflyState::Approving => {
                    ReflyState::Cancelled
                }
                ReflyState::Starting => ReflyState::Uncertain,
                other => other,
            };
        }
        let flying: Vec<(String, String)> = self
            .reflies
            .iter()
            .filter(|r| r.in_progress())
            .map(|r| (r.member.clone(), r.owner.clone()))
            .collect();
        for handover in &mut self.handovers {
            let started = flying.contains(&(handover.to.clone(), handover.from.clone()));
            if handover.state == State::Committed && !started {
                handover.state = State::Unassigned;
                handover.basis = format!(
                    "the team aborted before {} started the moved lines; an operator gives them to a member",
                    handover.to
                );
            }
        }
    }
}
