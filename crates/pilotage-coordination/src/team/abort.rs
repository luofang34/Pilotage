//! What a team abort ends (ADR-0052). The abort goes to every member, and
//! the team ends what it was still setting in motion: a re-flight or moved
//! lines not yet started are cancelled, and the receipts they wait for take
//! no next step. A flight whose start was sent may have started, so it
//! keeps its member and its uncertainty until the member reports. Committed
//! work that its receiver had not started is released and unassigned. A
//! release fact that arrives later is recorded, but it starts nothing.

use super::refly::ReflyState;
use super::{Op, TeamLane};
use pilotage_protocol::wire::MissionHandoverState as State;

impl TeamLane {
    /// Ends the work an abort of the team leaves pending.
    pub(super) fn end_pending_work(&mut self) {
        // Only the answer to a start already sent still tells something:
        // whether the member began the flight.
        let reflies = &self.reflies;
        self.pending.retain(|_, p| {
            p.op == Op::Start
                && p.refly
                    .and_then(|i| reflies.get(i))
                    .is_some_and(|r| r.state == ReflyState::Starting)
        });
        self.renewal = None;
        let mut cancelled = Vec::new();
        for refly in &mut self.reflies {
            refly.state = match refly.state {
                ReflyState::Queued | ReflyState::Proposing | ReflyState::Approving => {
                    if !refly.moved.is_empty() {
                        cancelled.push((refly.member.clone(), refly.owner.clone()));
                    }
                    ReflyState::Cancelled
                }
                ReflyState::Starting => ReflyState::Uncertain,
                other => other,
            };
        }
        for handover in &mut self.handovers {
            let never_started = cancelled.contains(&(handover.to.clone(), handover.from.clone()));
            if handover.state == State::Committed && never_started {
                handover.state = State::Unassigned;
                handover.basis = format!(
                    "the team aborted before {} started the moved lines; an operator gives them to a member",
                    handover.to
                );
            }
        }
    }
}
