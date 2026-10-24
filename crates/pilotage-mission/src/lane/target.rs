//! What an approval is for (ADR-0052), from its digest and its run: the
//! plan before its start, a drafted successor, or a renewal of the plan in
//! progress. A vehicle lane and a team resolve it with the same rule.

use pilotage_protocol::wire;

/// What an approval is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalTarget {
    /// The plan, before its start.
    Plan,
    /// A drafted successor of the plan in progress.
    Successor,
    /// The plan in progress, for its run.
    Renewal,
}

/// What a lane holds that an approval can be for.
#[derive(Debug, Clone, Copy)]
pub struct Approvable<'a> {
    /// The digest of the plan.
    pub plan: &'a [u8],
    /// The run in progress, when there is one.
    pub run: Option<&'a str>,
    /// The digest of a drafted successor, when there is one.
    pub drafted: Option<&'a [u8]>,
}

/// What `approval` is for, of what the lane holds.
///
/// # Errors
///
/// Why the approval is for nothing the lane holds: another digest, a run
/// that is not in progress, or a renewal that names no run.
pub fn approval_target(
    approval: &wire::MissionApproval,
    held: Approvable<'_>,
) -> Result<ApprovalTarget, String> {
    let digest = approval.proposal_digest.as_slice();
    let names = approval.run_id.as_str();
    let Some(run) = held.run else {
        if digest != held.plan {
            return Err("the approval binds another proposal digest".to_owned());
        }
        if !names.is_empty() {
            return Err(format!(
                "the approval names run {names}; no run is in progress"
            ));
        }
        return Ok(ApprovalTarget::Plan);
    };
    if held.drafted == Some(digest) {
        if !names.is_empty() && names != run {
            return Err(format!("the approval names run {names}; the run is {run}"));
        }
        return Ok(ApprovalTarget::Successor);
    }
    if digest == held.plan {
        if names != run {
            return Err(format!(
                "a renewal names the run it renews; the run is {run}, the approval names {}",
                if names.is_empty() { "none" } else { names }
            ));
        }
        return Ok(ApprovalTarget::Renewal);
    }
    Err("no plan or drafted revision has that digest".to_owned())
}
