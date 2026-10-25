//! Planned changes to a running team plan (ADR-0052): a revision is drafted
//! for review and applied through the members once an approval admits it,
//! or at once inside the team's envelope when the sender asked.
//!
//! The team never splices a member's run. A member that keeps fewer lines
//! gets a revision of its own, which it applies at the end of its line in
//! progress; lines moved to another member wait until that member
//! completes its own plan.

use pilotage_mission::IntentContext;
use pilotage_mission::change::{ChangeClass, ChangeSet, authorize};
use pilotage_mission::survey::objective_change;
use pilotage_protocol::wire;
use wire::MissionSuccessorState as Stage;

use super::intents::{delegate, valid_envelope};
use super::{MemberIntent, Op, Phase, TeamLane};

mod draft;

/// What a revision does to the members.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Change {
    /// `from` keeps `kept`; `to` flies `moved` after its plan.
    Reassign {
        from: String,
        to: String,
        kept: Vec<Vec<wire::GeoPoint>>,
        moved: Vec<Vec<wire::GeoPoint>>,
    },
    /// Each listed member keeps only these unflown lines; none ends it.
    Keep(Vec<(String, Vec<Vec<wire::GeoPoint>>)>),
}

/// A revision of the team plan in progress.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Revision {
    pub change: Change,
    pub stage: Stage,
    pub proposal: wire::MissionProposal,
    /// The draft as it travels.
    pub record: wire::MissionProposalRecord,
    pub drafted_by: String,
    /// The parent team whose revision this is the team's share of.
    pub share_of: String,
}

type Drafted = Result<(wire::MissionProposalRecord, Vec<MemberIntent>), String>;

impl TeamLane {
    pub(super) fn revise(
        &mut self,
        revise: &wire::MissionRevise,
        context: &IntentContext,
    ) -> Drafted {
        use wire::mission_revise::Change as Asked;
        let current = self
            .proposal
            .as_ref()
            .ok_or("there is no team plan to revise")?;
        if revise.base_digest != current.digest {
            return Err("the revision continues another team plan; the plan changed".to_owned());
        }
        if !matches!(self.phase, Phase::Running | Phase::Held) {
            return Err(format!(
                "a team revises a running plan; the team is {:?}: propose instead",
                self.phase
            ));
        }
        if self.run_id.as_deref() != Some(revise.run_id.as_str()) {
            return Err("the revision continues another team run".to_owned());
        }
        let asked = revise
            .change
            .as_ref()
            .ok_or("the revision names no change")?;
        let (change, summary) = match asked {
            Asked::Reassign(reassign) => self.draft_reassign(reassign)?,
            Asked::Finish(_) => self.draft_keep_none(),
            Asked::Objective(objective) => self.draft_keep(objective)?,
        };
        let changes = self.changes(asked, &change);
        let (draft, proposal) = self
            .revision_proposal(&change, summary, context)
            .into_parts();
        self.revision = Some(Revision {
            change,
            stage: Stage::Drafted,
            proposal: draft,
            record: proposal.clone(),
            drafted_by: revise
                .drafted_by
                .as_ref()
                .map(|o| o.subject.clone())
                .unwrap_or_default(),
            share_of: revise.share_of.clone(),
        });
        self.detail = "revision drafted; waiting for review".to_owned();
        if !revise.within_envelope {
            return Ok((proposal, Vec::new()));
        }
        let approval = self
            .approval
            .clone()
            .ok_or("the team runs without an approval")?;
        let permitted = valid_envelope(&approval, context.now_unix_ms)
            .map(|e| authorize(e, &changes).map_err(|unpermitted| unpermitted.to_string()));
        match permitted {
            Ok(Ok(())) => Ok((proposal, self.apply_revision(context))),
            Ok(Err(unpermitted)) => {
                self.detail = format!("{unpermitted}; the draft waits for an approval");
                Ok((proposal, Vec::new()))
            }
            Err(reason) => {
                self.detail = format!("{reason}; the draft waits for an approval");
                Ok((proposal, Vec::new()))
            }
        }
    }

    /// Every change the drafted `change` makes, from comparing the team's
    /// work before and after it, not from the kind of request.
    fn changes(&self, asked: &wire::mission_revise::Change, change: &Change) -> ChangeSet {
        use wire::mission_revise::Change as Asked;
        let mut set = ChangeSet::default();
        if let (Asked::Objective(after), Some(before)) = (asked, self.objective.as_ref()) {
            set.extend(objective_change::classify(before, after));
        }
        match change {
            Change::Reassign {
                from, to, moved, ..
            } => {
                set.add(
                    ChangeClass::Assignment,
                    format!("{} lines move from {from} to {to}", moved.len()),
                );
                set.add(ChangeClass::Plan, "the members' lines");
            }
            Change::Keep(_) => set.add(ChangeClass::Plan, "the members' lines"),
        }
        set
    }

    /// Answers a revise intent: before a start a revised objective is a
    /// new proposal; while running, the draft for review.
    pub(super) fn on_revise(
        &mut self,
        id: String,
        revise: &wire::MissionRevise,
        context: &IntentContext,
    ) -> super::TeamReply {
        use wire::mission_revise::Change as Asked;
        let before_start = matches!(self.phase, Phase::Proposed | Phase::Admitted);
        let current = self
            .proposal
            .as_ref()
            .map(|p| p.digest.clone())
            .unwrap_or_default();
        let result = match (before_start, revise.change.as_ref()) {
            (true, Some(Asked::Objective(objective))) if revise.base_digest == current => {
                match self.propose(objective, context) {
                    Ok((intents, _)) => {
                        self.operator_request = Some(id);
                        return super::TeamReply {
                            receipt: None,
                            intents,
                        };
                    }
                    Err(reason) => Err(reason),
                }
            }
            (true, _) => {
                Err("before a start, only a revised objective replaces the team plan".to_owned())
            }
            (false, _) => self.revise(revise, context),
        };
        match result {
            Ok((proposal, intents)) => super::TeamReply {
                receipt: Some(self.receipt(id, true, String::new(), Some(proposal))),
                intents,
            },
            Err(reason) => super::TeamReply {
                receipt: Some(self.receipt(id, false, reason, None)),
                intents: Vec::new(),
            },
        }
    }

    /// A request for a member's share of the revision.
    fn request_revision(
        &mut self,
        member: &str,
        op: Op,
        action: wire::mission_intent::Action,
    ) -> MemberIntent {
        let intent = self.request(member, op, None, action);
        if let Some(pending) = self.pending.get_mut(&intent.intent.request_id) {
            pending.revision = true;
        }
        intent
    }

    /// An approval of the drafted revision's digest.
    pub(super) fn approve_revision(
        &mut self,
        approval: &wire::MissionApproval,
        context: &IntentContext,
    ) -> Result<Vec<MemberIntent>, String> {
        let drafted = self
            .revision
            .as_ref()
            .filter(|r| r.stage == Stage::Drafted && r.proposal.digest == approval.proposal_digest)
            .ok_or("no drafted revision has that digest")?;
        super::intents::bound_format(approval)?;
        if let Change::Reassign { from, .. } = &drafted.change
            && self.lost.get(from) == Some(&wire::MemberLink::Lost)
        {
            return Err(format!(
                "{from}'s link is lost and its state is unknown; its work stays reserved until it reports again or a person states that it is down"
            ));
        }
        let envelope = valid_envelope(approval, context.now_unix_ms)?;
        if self.height_m > envelope.max_height_m + 1e-6 {
            return Err(format!(
                "the team flies at {:.1} m; the envelope allows {:.1} m",
                self.height_m, envelope.max_height_m
            ));
        }
        self.approval = Some(approval.clone());
        Ok(self.apply_revision(context))
    }

    /// Sends each member the change that is its share of the revision.
    fn apply_revision(&mut self, context: &IntentContext) -> Vec<MemberIntent> {
        let Some(revision) = self.revision.as_mut() else {
            return Vec::new();
        };
        revision.stage = Stage::Applied;
        let summary = revision.proposal.summary.clone();
        let change = revision.change.clone();
        self.detail = format!("applying the revision: {summary}");
        let mut intents = Vec::new();
        match change {
            Change::Reassign {
                from,
                to,
                kept,
                moved,
            } => {
                if self.stated_down(&from) {
                    self.lost.insert(from.clone(), wire::MemberLink::Released);
                    // A member a person stated down hears nothing: its lines
                    // are skipped here, and the team no longer waits for it.
                    self.book.skip_paths(&from, &moved);
                    if let Some(status) = self.statuses.get_mut(&from) {
                        status.detail = format!("down; {to} flies its lines");
                    }
                    self.detail = format!(
                        "{from} is down; {to} flies its {} unfinished lines",
                        moved.len()
                    );
                    let assignment = self.override_handover(&from, &to, &moved);
                    intents.extend(self.queue_moved(&to, &from, moved, assignment, context));
                } else {
                    // The moved lines go to `to` only once `from` reports
                    // that it released them.
                    let release = self.open_handover(&from, &to, &moved);
                    intents.extend(self.member_revision(&from, &kept, context, Some(release)));
                    self.detail = format!("waiting for {from} to release the moved lines");
                }
            }
            Change::Keep(keep) => {
                for (member, kept) in keep {
                    intents.extend(self.member_revision(&member, &kept, context, None));
                }
            }
        }
        intents
    }

    /// The revision that leaves `member` only `kept` of its unflown lines,
    /// or ends it after its line in progress when none stays.
    fn member_revision(
        &mut self,
        member: &str,
        kept: &[Vec<wire::GeoPoint>],
        context: &IntentContext,
        release: Option<wire::WorkRelease>,
    ) -> Vec<MemberIntent> {
        use wire::mission_revise::Change as Asked;
        let Some(status) = self.statuses.get(member).cloned() else {
            return Vec::new();
        };
        let change = if kept.is_empty() {
            Asked::Finish(wire::MissionFinish {})
        } else {
            let (Some(across), Some(sweep)) = (self.across(), self.sweep) else {
                return Vec::new();
            };
            let area = match self.objective.as_ref().and_then(|o| o.kind.as_ref()) {
                Some(wire::mission_objective::Kind::Survey(s)) => s.area.clone(),
                None => return Vec::new(),
            };
            let positions: Vec<f64> = kept.iter().filter_map(|p| across.of_path(p)).collect();
            let strip = across.strip(&area, &positions, sweep.footprint_m);
            let Some(objective) = self.member_objective(
                member,
                strip,
                format!("{}/revised/{member}", context.fresh_id),
                format!("{member}: {} lines kept", kept.len()),
            ) else {
                return Vec::new();
            };
            Asked::Objective(objective)
        };
        let revise = wire::MissionRevise {
            base_digest: status.proposal_digest.clone(),
            run_id: status.run_id.clone(),
            within_envelope: false,
            drafted_by: None,
            release,
            share_of: self.config.name.clone(),
            change: Some(change),
        };
        vec![self.request_revision(
            member,
            Op::Revise,
            wire::mission_intent::Action::Revise(revise),
        )]
    }

    /// A member's answer to its share of the revision: an accepted draft
    /// gets the team's approval, delegated to the member's own draft.
    pub(super) fn on_revision_receipt(
        &mut self,
        member: &str,
        op: Op,
        accepted: bool,
        receipt: &wire::MissionReceipt,
    ) -> Vec<MemberIntent> {
        if !accepted {
            self.detail = format!(
                "{member} refused its share of the revision: {}",
                receipt.reason
            );
            self.withdraw_handover(member, &receipt.reason);
            return Vec::new();
        }
        if op != Op::Revise {
            return Vec::new();
        }
        let draft = match super::opened(receipt) {
            Ok(sealed) => sealed.proposal().clone(),
            Err(reason) => {
                self.detail = format!("{member}'s share of the revision: {reason}");
                return Vec::new();
            }
        };
        let Some(team) = self.approval.clone() else {
            return Vec::new();
        };
        if self.keeps_moved_work(member, &draft) {
            self.detail = format!("{member}'s share still flies the moved lines; not approved");
            self.withdraw_handover(member, "its share still flies the moved lines");
            return Vec::new();
        }
        self.approving_release(member, &draft.digest);
        let band: Vec<wire::GeoPoint> = draft.tasks.iter().flat_map(|t| t.path.clone()).collect();
        let launch = self.member(member).map(|m| m.launch);
        let approval = delegate(
            &team,
            &draft.proposal_id,
            &draft.digest,
            &band,
            launch,
            &self.config.name,
        );
        let approval = self.with_share(member, approval);
        vec![self.request_revision(
            member,
            Op::Approve,
            wire::mission_intent::Action::Approve(approval),
        )]
    }

    /// A person states that `member`, whose link is lost, is down. Only then
    /// may its unfinished work move, its line in progress included; until
    /// then the work stays reserved, as the member may still fly it. A
    /// member that is down stays down, and its later reports are not taken.
    pub(super) fn member_down(
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
                Ok((Vec::new(), false))
            }
        }
    }

    /// A member's link is gone: its state is unknown, and its work and its
    /// line in progress stay reserved to it until it reports again or a
    /// person states that it is down.
    pub fn on_member_lost(&mut self, member: &str) {
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
    pub(super) fn reporting(&mut self, member: &str) -> bool {
        if self.stated_down(member) {
            return false;
        }
        self.lost.remove(member);
        true
    }

    /// Whether a person stated `member` down, whether or not its work moved.
    pub(super) fn stated_down(&self, member: &str) -> bool {
        matches!(
            self.lost.get(member),
            Some(wire::MemberLink::Down | wire::MemberLink::Released)
        )
    }

    /// How the team sees `member`.
    pub(super) fn link(&self, member: &str) -> wire::MemberLink {
        self.lost
            .get(member)
            .copied()
            .unwrap_or(wire::MemberLink::Reporting)
    }

    /// The revision as the team's status reports it.
    pub(super) fn revision_status(&self) -> Option<wire::MissionSuccessor> {
        let revision = self.revision.as_ref()?;
        Some(wire::MissionSuccessor {
            digest: revision.proposal.digest.clone(),
            summary: revision.proposal.summary.clone(),
            state: revision.stage as i32,
            applies_after: match revision.stage {
                Stage::Applied => String::new(),
                _ => "each member's line in progress".to_owned(),
            },
            drafted_by: revision.drafted_by.clone(),
        })
    }
}
