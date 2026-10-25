//! The start of an extra flight: at once while the team runs, or, for one
//! approved while the team holds, once the team resumes.

use pilotage_protocol::wire;

use super::super::{MemberIntent, Op, TeamLane};
use super::ReflyState;

impl TeamLane {
    /// Starts the approved extra flight `index`.
    pub(super) fn start_refly(&mut self, index: usize) -> Vec<MemberIntent> {
        let Some(refly) = self.reflies.get_mut(index) else {
            return Vec::new();
        };
        let member = refly.member.clone();
        refly.state = ReflyState::Starting;
        refly.prior_run = self.statuses.get(&member).map(|s| s.run_id.clone());
        let digest = refly
            .proposal
            .as_ref()
            .map(|p| p.digest.clone())
            .unwrap_or_default();
        vec![self.request(
            &member,
            Op::Start,
            Some(index),
            wire::mission_intent::Action::Start(wire::MissionStart {
                proposal_digest: digest,
            }),
        )]
    }

    /// Starts each extra flight approved while the team held.
    pub(in crate::team) fn start_approved_reflies(&mut self) -> Vec<MemberIntent> {
        let approved: Vec<usize> = self
            .reflies
            .iter()
            .enumerate()
            .filter(|(_, r)| r.state == ReflyState::Approved)
            .map(|(index, _)| index)
            .collect();
        approved
            .into_iter()
            .flat_map(|index| self.start_refly(index))
            .collect()
    }
}
