//! A team's members' links (ADR-0052): a member whose link is lost keeps
//! its work reserved until it reports again or a person states it down.

use pilotage_protocol::wire;

use super::super::{MemberIntent, Phase, TeamLane};

impl TeamLane {
    /// A person states that `member`, whose link is lost, is down. Only then
    /// may its unfinished work move, its line in progress included; until
    /// then the work stays reserved, as the member may still fly it. A
    /// member that is down stays down, and its later reports are not taken.
    pub(in crate::team) fn member_down(
        &mut self,
        down: &wire::MissionMemberDown,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        let member = down.member.as_str();
        match self.lost.get(member) {
            None if self.member(member).is_none() => {
                Err(format!("{member} is not a member of {}", self.config.name))
            }
            None => Err(format!(
                "{member} reports; only a member whose link is lost can be stated down"
            )),
            Some(wire::MemberLink::Released) => Ok((Vec::new(), false)),
            Some(_) => {
                self.lost.insert(member.to_owned(), wire::MemberLink::Down);
                let stated_by = down
                    .stated_by
                    .as_ref()
                    .map_or_else(|| "a person".to_owned(), |o| o.subject.clone());
                self.stated_by.insert(member.to_owned(), stated_by);
                self.unassign_queued_for(member);
                if let Some(status) = self.statuses.get_mut(member) {
                    status.detail = "down, as a person stated; its work may move".to_owned();
                }
                self.detail = format!("{member} is down; reassign its lines to another member");
                if self.phase == Phase::Settling {
                    // Its open evidence no longer keeps the team waiting,
                    // and no other member may report again to end it.
                    self.settle();
                }
                Ok((Vec::new(), false))
            }
        }
    }

    /// A member's link is gone: its state is unknown, and its work and its
    /// line in progress stay reserved to it until it reports again or a
    /// person states that it is down.
    pub fn on_member_lost(&mut self, member: &str) {
        self.lose_answers_of(member);
        if self.stated_down(member) {
            return;
        }
        if let Some(status) = self.statuses.get_mut(member) {
            status.detail = "link lost; state unknown, work stays reserved".to_owned();
            self.changed = true;
        }
        self.lost
            .entry(member.to_owned())
            .or_insert(wire::MemberLink::Lost);
    }

    /// Whether the team takes `member`'s report: a member that reports
    /// again is no longer lost, unless a person stated it down, as others
    /// may fly its lines.
    pub(in crate::team) fn reporting(&mut self, member: &str) -> bool {
        if self.stated_down(member) {
            return false;
        }
        self.lost.remove(member);
        true
    }

    /// Whether a person stated `member` down, whether or not its work moved.
    pub(in crate::team) fn stated_down(&self, member: &str) -> bool {
        matches!(
            self.lost.get(member),
            Some(wire::MemberLink::Down | wire::MemberLink::Released)
        )
    }

    /// How the team sees `member`.
    pub(in crate::team) fn link(&self, member: &str) -> wire::MemberLink {
        self.lost
            .get(member)
            .copied()
            .unwrap_or(wire::MemberLink::Reporting)
    }
}
