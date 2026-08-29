use crate::{
    AttemptRole, Candidate, CandidateTransitionReceipt, CandidateTransitionReference,
    CandidateTransitionRequest, Digest, Journal, JournalEvent, SearchGroupBinding, SearchStage,
    SimulatorVehicleAdapter, TuneError, VehicleBinding,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn authorize_new<V>(
    journal: &mut Journal,
    stage: &SearchStage,
    vehicle: &VehicleBinding<V>,
    source: &Candidate,
    target: &Candidate,
    attempt_index: u64,
    reason: &str,
    group: &SearchGroupBinding,
) -> Result<CandidateTransitionReference, TuneError>
where
    V: SimulatorVehicleAdapter,
{
    let source_digest = super::evaluate::candidate_digest(source)?;
    let target_digest = super::evaluate::candidate_digest(target)?;
    let request = transition_request(
        journal,
        stage,
        source,
        source_digest,
        target,
        target_digest,
        attempt_index,
        group,
    )?;
    journal.ensure_usable()?;
    let receipt = authorize_with_vehicle(journal, vehicle, &request)?;
    journal.ensure_usable()?;
    journal.authorize_training_transition(attempt_index, reason, target, group, receipt)
}

pub(super) fn reauthorize_saved<V>(
    journal: &Journal,
    stage: &SearchStage,
    vehicle: &VehicleBinding<V>,
) -> Result<(), TuneError>
where
    V: SimulatorVehicleAdapter,
{
    let authorizations = saved_authorizations(journal);
    for (attempt_index, group, receipt) in authorizations {
        journal.ensure_usable()?;
        let source = journal.read_candidate(receipt.source_candidate_digest())?;
        let target = journal.read_candidate(receipt.target_candidate_digest())?;
        let request = transition_request(
            journal,
            stage,
            &source,
            receipt.source_candidate_digest(),
            &target,
            receipt.target_candidate_digest(),
            attempt_index,
            &group,
        )?;
        receipt.validate_for(&request)?;
        let observed = authorize_with_vehicle(journal, vehicle, &request)?;
        if observed != receipt {
            return Err(TuneError::ReceiptMismatch {
                operation: "reauthorize candidate transition",
                detail: "the vehicle did not reproduce the saved authorization".to_owned(),
            });
        }
    }
    journal.ensure_usable()
}

#[allow(clippy::too_many_arguments)]
fn transition_request(
    journal: &Journal,
    stage: &SearchStage,
    source: &Candidate,
    source_digest: Digest,
    target: &Candidate,
    target_digest: Digest,
    attempt_index: u64,
    group: &SearchGroupBinding,
) -> Result<CandidateTransitionRequest, TuneError> {
    let role = AttemptRole::TrainingChallenger {
        attempt_index,
        suite_index: group.suite_index,
    };
    let plan_digest = role.plan_digest(stage, target_digest, journal.session().fixed_seed)?;
    let planning_context = crate::adapter::planning_context_digest(
        journal.session().stage_digest,
        plan_digest,
        group,
    )?;
    CandidateTransitionRequest::new(
        journal.session_digest()?,
        source,
        source_digest,
        target,
        target_digest,
        journal.session().runtimes.transition_validator.clone(),
        journal.session().runtimes.adjacency_policy_digest,
        planning_context,
    )
}

fn authorize_with_vehicle<V>(
    journal: &Journal,
    vehicle: &VehicleBinding<V>,
    request: &CandidateTransitionRequest,
) -> Result<CandidateTransitionReceipt, TuneError>
where
    V: SimulatorVehicleAdapter,
{
    vehicle
        .authorize_candidate_transition(request)
        .map_err(|source| TuneError::Adapter {
            adapter: journal.session().runtimes.transition_validator.id.clone(),
            operation: "authorize candidate transition",
            source,
        })
}

fn saved_authorizations(
    journal: &Journal,
) -> Vec<(u64, SearchGroupBinding, CandidateTransitionReceipt)> {
    journal
        .entries()
        .iter()
        .filter_map(|entry| match &entry.event {
            JournalEvent::CandidateTransitionAuthorized {
                attempt_index,
                group,
                receipt,
                ..
            } => Some((*attempt_index, group.clone(), receipt.clone())),
            _ => None,
        })
        .collect()
}
