//! Planned changes to a running team plan (ADR-0052): a revision is drafted
//! for review and applied through the members once an approval admits it,
//! or at once inside the team's envelope when the sender asked.
//!
//! The team never splices a member's run. A member that keeps fewer lines
//! gets a revision of its own, which it applies at the end of its line in
//! progress; lines moved to another member wait until that member
//! completes its own plan.

use pilotage_mission::IntentContext;
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
    pub drafted_by: String,
}

type Drafted = Result<(wire::MissionProposal, Vec<MemberIntent>), String>;

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
        let (change, summary) = match revise
            .change
            .as_ref()
            .ok_or("the revision names no change")?
        {
            Asked::Reassign(reassign) => self.draft_reassign(reassign)?,
            Asked::Finish(_) => self.draft_keep_none(),
            Asked::Objective(objective) => self.draft_keep(objective)?,
        };
        let proposal = self.revision_proposal(&change, summary, context);
        self.revision = Some(Revision {
            change,
            stage: Stage::Drafted,
            proposal: proposal.clone(),
            drafted_by: revise
                .drafted_by
                .as_ref()
                .map(|o| o.subject.clone())
                .unwrap_or_default(),
        });
        self.detail = "revision drafted; waiting for review".to_owned();
        if !revise.within_envelope {
            return Ok((proposal, Vec::new()));
        }
        let approval = self
            .approval
            .clone()
            .ok_or("the team runs without an approval")?;
        let reassigns = matches!(
            self.revision.as_ref().map(|r| &r.change),
            Some(Change::Reassign { .. })
        );
        let permitted = valid_envelope(&approval, context.now_unix_ms)
            .map(|e| e.permit_revise && (e.permit_allocate || !reassigns));
        match permitted {
            Ok(true) => Ok((proposal, self.apply_revision(context))),
            Ok(false) => {
                self.detail = "the approval does not permit this change inside its envelope; the draft waits for an approval".to_owned();
                Ok((proposal, Vec::new()))
            }
            Err(reason) => {
                self.detail = format!("{reason}; the draft waits for an approval");
                Ok((proposal, Vec::new()))
            }
        }
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
        let _ = drafted;
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
                if self.lost.contains_key(&from) {
                    // A lost member hears nothing: its lines are skipped
                    // here, and the team no longer waits for it.
                    self.book.skip_paths(&from, &moved);
                    self.lost.insert(from.clone(), true);
                    self.detail = format!(
                        "{from} released: its link is lost and its state unknown; {to} flies its {} unfinished lines",
                        moved.len()
                    );
                } else {
                    intents.extend(self.member_revision(&from, &kept, context));
                }
                intents.extend(self.queue_moved(&to, &from, moved, context));
            }
            Change::Keep(keep) => {
                for (member, kept) in keep {
                    intents.extend(self.member_revision(&member, &kept, context));
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
            return Vec::new();
        }
        if op != Op::Revise {
            return Vec::new();
        }
        let (Some(draft), Some(team)) = (receipt.proposal.clone(), self.approval.clone()) else {
            return Vec::new();
        };
        let band: Vec<wire::GeoPoint> = draft.tasks.iter().flat_map(|t| t.path.clone()).collect();
        let launch = self.member(member).map(|m| m.launch);
        let approval = delegate(&team, &draft, &band, launch, &self.config.name);
        vec![self.request_revision(
            member,
            Op::Approve,
            wire::mission_intent::Action::Approve(approval),
        )]
    }

    /// A member's link is gone: its state is unknown, and its work stays
    /// reserved to it until a person reassigns that work, which releases
    /// the member.
    pub fn on_member_lost(&mut self, member: &str) {
        if let Some(status) = self.statuses.get_mut(member) {
            status.detail = "link lost; state unknown, work stays reserved".to_owned();
            self.changed = true;
        }
        self.lost.entry(member.to_owned()).or_insert(false);
    }

    /// Whether the team takes `member`'s report: a member that reports
    /// again is no longer lost, unless a person released it, whose lines
    /// others now fly.
    pub(super) fn reporting(&mut self, member: &str) -> bool {
        if self.lost.get(member) == Some(&true) {
            return false;
        }
        self.lost.remove(member);
        true
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
