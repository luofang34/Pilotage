//! What an approval is for (ADR-0052), from its digest and its run: the
//! plan before its start, a drafted successor, or a renewal of the plan in
//! progress. A vehicle lane and a team resolve it with the same rule, and
//! admit it with the same checks before either changes anything: it names
//! the record format whose digest it binds, it permits execution, and it
//! has not expired. Limits of the domain, such as height or area, stay with
//! the domain's own check.

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

/// What `approval` is for, of what the lane holds, once it passes the
/// checks every entry point makes. Returns its envelope too.
///
/// # Errors
///
/// Why the approval is for nothing the lane holds, or why it is not valid
/// now: see [`approval_target`] and [`valid_approval`].
pub fn admit_approval<'a>(
    approval: &'a wire::MissionApproval,
    held: Approvable<'_>,
    now_unix_ms: u64,
) -> Result<(ApprovalTarget, &'a wire::AdaptationEnvelope), String> {
    let target = approval_target(approval, held)?;
    Ok((target, valid_approval(approval, now_unix_ms)?))
}

/// The envelope of `approval`, if the approval names the record format
/// whose digest it binds, permits execution and has not expired at
/// `now_unix_ms`.
///
/// # Errors
///
/// Which of these the approval fails.
pub fn valid_approval(
    approval: &wire::MissionApproval,
    now_unix_ms: u64,
) -> Result<&wire::AdaptationEnvelope, String> {
    // A client that names no format does not know records, and the digest
    // it holds may not name the bytes the lane compiled.
    if approval.proposal_format != wire::MissionProposalFormat::V1 as i32 {
        return Err("the approval names no proposal record format; the client must approve a proposal record".to_owned());
    }
    still_valid(approval, now_unix_ms)
}

/// The envelope of an approval already admitted, if it still permits
/// execution and has not expired at `now_unix_ms`.
pub(super) fn still_valid(
    approval: &wire::MissionApproval,
    now_unix_ms: u64,
) -> Result<&wire::AdaptationEnvelope, String> {
    let envelope = approval
        .envelope
        .as_ref()
        .ok_or("the approval has no adaptation envelope")?;
    if !envelope.permit_execute {
        return Err("the approval does not permit execution".to_owned());
    }
    if envelope.expires_unix_ms == 0 || envelope.expires_unix_ms <= now_unix_ms {
        return Err("the approval has no expiry in the future".to_owned());
    }
    Ok(envelope)
}
