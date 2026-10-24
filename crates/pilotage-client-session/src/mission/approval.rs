//! The approval a client grants a proposal, and the adaptation envelope it
//! carries. Every client starts from the same default policy: a little
//! height above the plan's, an hour, execution with revision and
//! allocation but no change of objective, a few re-flown lines, a fifth of
//! the battery kept in reserve, and a hold in place if the hour ends during
//! the run.

use pilotage_protocol::wire;
use serde::{Deserialize, Serialize};

/// What an approval permits. A field absent from the JSON form keeps its
/// default.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApprovalPolicy {
    /// The highest height the lanes may fly, metres; absent takes the
    /// objective's height plus [`Self::height_margin_m`].
    pub max_height_m: Option<f64>,
    /// Height above the objective's that the lanes may climb, metres.
    pub height_margin_m: f64,
    /// How long the approval admits anything, minutes.
    pub expires_min: u64,
    /// Survey lines a lane may fly again without a new approval.
    pub max_refly_lines: u32,
    /// The lanes may revise the plan inside the envelope.
    pub permit_revise: bool,
    /// The least battery fraction at which the plan must end.
    pub min_reserve_fraction: f64,
    /// What a run does when the approval expires while it flies.
    pub on_expiry: OnExpiry,
}

/// What a run does when its approval expires while it flies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExpiry {
    /// Stop in place and wait for a new approval.
    #[default]
    Hold,
    /// End after the line in progress and return to the launch point.
    ReturnHome,
}

impl OnExpiry {
    fn wire(self) -> wire::MissionExpiryPolicy {
        match self {
            Self::Hold => wire::MissionExpiryPolicy::Hold,
            Self::ReturnHome => wire::MissionExpiryPolicy::ReturnHome,
        }
    }
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            max_height_m: None,
            height_margin_m: 10.0,
            expires_min: 60,
            max_refly_lines: 4,
            permit_revise: true,
            min_reserve_fraction: 0.2,
            on_expiry: OnExpiry::Hold,
        }
    }
}

/// The highest height `policy` allows for `proposal`: the stated ceiling,
/// else the compiled objective's height plus the margin; `None` when
/// neither states a height, as the margin alone bounds nothing.
#[must_use]
pub fn approved_height_m(proposal: &wire::MissionProposal, policy: &ApprovalPolicy) -> Option<f64> {
    let positive = |height: &f64| height.is_finite() && *height > 0.0;
    if let Some(ceiling) = policy.max_height_m.filter(positive) {
        return Some(ceiling);
    }
    proposal
        .objective
        .as_ref()
        .and_then(|objective| objective.kind.as_ref())
        .map(|wire::mission_objective::Kind::Survey(survey)| survey.height_m)
        .filter(positive)
        .map(|height| height + policy.height_margin_m)
}

/// The approval of `proposal` at `now_ms` under `policy`, or `None` when
/// no height bounds it. The host writes the approver; the record names the
/// session that sent it.
#[must_use]
pub fn approval(
    proposal: &wire::MissionProposal,
    policy: &ApprovalPolicy,
    now_ms: u64,
) -> Option<wire::mission_intent::Action> {
    let max_height_m = approved_height_m(proposal, policy)?;
    let expires_ms = policy.expires_min.saturating_mul(60_000);
    Some(wire::mission_intent::Action::Approve(
        wire::MissionApproval {
            approval_id: format!("approval-{now_ms}"),
            proposal_digest: proposal.digest.clone(),
            approver: None,
            envelope: Some(wire::AdaptationEnvelope {
                max_height_m,
                min_reserve_fraction: policy.min_reserve_fraction,
                expires_unix_ms: now_ms.saturating_add(expires_ms),
                bound: Vec::new(),
                permit_execute: true,
                permit_revise: policy.permit_revise,
                permit_allocate: true,
                permit_change_objective: false,
                max_refly_lines: policy.max_refly_lines,
                on_expiry: policy.on_expiry.wire() as i32,
            }),
            approved_unix_ms: now_ms,
            proposal_format: wire::MissionProposalFormat::V1 as i32,
        },
    ))
}
