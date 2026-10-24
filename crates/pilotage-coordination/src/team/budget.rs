//! The team's re-flight budget, divided among those that spend it (ADR-0052).
//! A member team decides the re-flights of its own lines, so it gets an even
//! part of the budget; the first member teams by name get one more line
//! each until the remainder is gone. When the team also has vehicle members,
//! it keeps one part for their lines. A vehicle member decides no re-flight,
//! so it gets none. Delegation only divides the budget: the parts never add
//! up to more than the team's own approval allows.

use pilotage_protocol::wire;

use super::TeamLane;

impl TeamLane {
    /// Whether `member` is a team: its proposal binds member proposals.
    fn member_is_team(&self, member: &str) -> bool {
        self.member_proposals
            .get(member)
            .is_some_and(|p| !p.members.is_empty())
    }

    fn refly_budget(&self) -> u32 {
        self.approval
            .as_ref()
            .and_then(|a| a.envelope.as_ref())
            .map_or(0, |e| e.max_refly_lines)
    }

    /// The re-flights `member` may decide on its own.
    pub(super) fn refly_share(&self, member: &str) -> u32 {
        if !self.member_is_team(member) {
            return 0;
        }
        let teams: Vec<&String> = self
            .member_proposals
            .keys()
            .filter(|m| self.member_is_team(m))
            .collect();
        let keeps = self
            .member_proposals
            .keys()
            .any(|m| !self.member_is_team(m));
        let parts = u32::try_from(teams.len()).unwrap_or(u32::MAX) + u32::from(keeps);
        let total = self.refly_budget();
        let (base, extra) = (total / parts.max(1), total % parts.max(1));
        let index = teams.iter().position(|m| m.as_str() == member);
        let index = index
            .and_then(|i| u32::try_from(i).ok())
            .unwrap_or(u32::MAX);
        base + u32::from(index < extra)
    }

    /// The re-flights the team decides itself, on its vehicle members' lines.
    pub(super) fn own_refly_budget(&self) -> u32 {
        let given: u32 = self
            .member_proposals
            .keys()
            .map(|m| self.refly_share(m))
            .sum();
        self.refly_budget().saturating_sub(given)
    }

    /// `approval`, passed down to `member`, with the member's part of the
    /// re-flight budget.
    pub(super) fn with_share(
        &self,
        member: &str,
        mut approval: wire::MissionApproval,
    ) -> wire::MissionApproval {
        if let Some(envelope) = approval.envelope.as_mut() {
            envelope.max_refly_lines = self.refly_share(member);
        }
        approval
    }
}
