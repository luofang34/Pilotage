use crate::journal::replay::{JournalState, invalid};
use crate::journal::transition::AuthorizedTrainingTransition;
use crate::journal::{AttemptRole, CampaignPhase, JournalEvent, SessionIdentity};
use crate::{
    CandidateTransitionReceipt, CandidateTransitionReference, Digest, SearchGroupBinding,
    SearchStage, TuneError,
};

#[cfg(test)]
#[path = "transition/tests.rs"]
mod tests;

pub(super) fn authorize_event(
    state: &mut JournalState,
    event: &JournalEvent,
    stage: &SearchStage,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    let JournalEvent::CandidateTransitionAuthorized {
        attempt_index,
        reason,
        candidate,
        group,
        receipt,
    } = event
    else {
        return Err(invalid("the event is not a transition authorization"));
    };
    authorize(
        state,
        *attempt_index,
        reason,
        *candidate,
        group,
        receipt,
        stage,
        session,
    )
}

/// Checks that one recorded binding states a group the frozen stage declares.
///
/// The check is complete for the stage: the group exists, it names the stated
/// suite, and the suite content identity is the recomputed one. The parameters
/// that the challenger changed are checked against the stored candidates when
/// the journal opens, because the chain carries digests and not parameters.
fn validate_group_binding(
    stage: &SearchStage,
    group: &SearchGroupBinding,
) -> Result<(), TuneError> {
    let declared = stage
        .search_groups
        .iter()
        .find(|candidate| candidate.id == group.group_id)
        .ok_or_else(|| invalid("a transition names a group the stage does not declare"))?;
    let suite = stage.training_suite(group.suite_index)?;
    if declared.suite_id != group.suite_id
        || suite.id != group.suite_id
        || suite.digest()? != group.suite_digest
    {
        return Err(invalid("a transition changed its derived training suite"));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn authorize(
    state: &mut JournalState,
    attempt_index: u64,
    reason: &str,
    candidate: Digest,
    group: &SearchGroupBinding,
    receipt: &CandidateTransitionReceipt,
    stage: &SearchStage,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    validate_group_binding(stage, group)?;
    if state.phase != CampaignPhase::Searching
        || !state.has_passed_suite_baseline(state.training_incumbent, group.suite_index)
        || state.pending.is_some()
        || state.authorized_transition.is_some()
        || attempt_index != state.training_attempt_count
        || reason.trim().is_empty()
        || reason.len() > 4_096
        || candidate.is_zero()
        || candidate == state.training_incumbent
        || state
            .training_history
            .iter()
            .any(|observation| observation.candidate_digest == candidate)
    {
        return Err(invalid(
            "candidate transition authorization has invalid campaign state",
        ));
    }
    let session_digest = super::super::storage::document_digest("session identity", session)?;
    let role = AttemptRole::TrainingChallenger {
        attempt_index,
        suite_index: group.suite_index,
    };
    let plan_digest = role.plan_digest(stage, candidate, session.fixed_seed)?;
    let expected_planning_context =
        crate::adapter::planning_context_digest(session.stage_digest, plan_digest, group)?;
    let reference = receipt.reference();
    reference
        .validate_for_runtime(
            session_digest,
            state.training_incumbent,
            candidate,
            &session.runtimes.transition_validator,
            session.runtimes.adjacency_policy_digest,
            expected_planning_context,
        )
        .map_err(|_| invalid("candidate transition reference is not valid during replay"))?;
    let recomputed = receipt
        .recompute_digest()
        .map_err(|_| invalid("candidate transition receipt cannot be recomputed"))?;
    if receipt.source_candidate_digest() != state.training_incumbent
        || receipt.target_candidate_digest() != candidate
        || receipt.validator() != &session.runtimes.transition_validator
        || receipt.adjacency_policy_digest() != session.runtimes.adjacency_policy_digest
        || receipt.planning_context_digest() != expected_planning_context
        || recomputed != receipt.receipt_digest()
    {
        return Err(invalid(
            "candidate transition receipt does not match the replay identity",
        ));
    }
    state.authorized_transition = Some(AuthorizedTrainingTransition {
        attempt_index,
        candidate,
        group: group.clone(),
        reference,
    });
    Ok(())
}

pub(super) fn validate_attempt(
    state: &JournalState,
    role: AttemptRole,
    candidate: Digest,
    reference: Option<&CandidateTransitionReference>,
) -> Result<(), TuneError> {
    match role {
        AttemptRole::TrainingChallenger {
            attempt_index,
            suite_index,
        } => {
            let authorized = state
                .authorized_transition
                .as_ref()
                .ok_or_else(|| invalid("a training challenger has no transition authorization"))?;
            if authorized.attempt_index != attempt_index
                || authorized.candidate != candidate
                || authorized.group.suite_index != suite_index
                || Some(&authorized.reference) != reference
            {
                return Err(invalid(
                    "a training challenger changed its transition authorization",
                ));
            }
        }
        AttemptRole::TrainingBaseline { .. }
        | AttemptRole::PromotionBaseline
        | AttemptRole::PromotionFrozen
        | AttemptRole::FinalQualification => {
            if reference.is_some() || state.authorized_transition.is_some() {
                return Err(invalid(
                    "a non-challenger attempt has a transition authorization",
                ));
            }
        }
    }
    Ok(())
}
