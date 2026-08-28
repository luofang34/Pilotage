//! The training prelude a sealed campaign's journal opens with, and
//! the authorized transition its challenger rides.

use flight_tune::{
    AttemptRole, AuthenticatedEvaluationProof, CandidateTransitionReference,
    CandidateTransitionRequest, Digest, SearchStage, SessionIdentity,
};

use super::{Point, fixed_digest, proof, tuning_candidate};

/// The training prelude: the baseline attempt, the authorized
/// transition, and the challenger that rides it.
pub(super) fn training_attempts(
    stage: &SearchStage,
    session: &SessionIdentity,
    session_digest: Digest,
    frozen_candidate: Digest,
) -> (
    AuthenticatedEvaluationProof,
    (
        flight_tune::CandidateTransitionReceipt,
        CandidateTransitionReference,
    ),
    AuthenticatedEvaluationProof,
) {
    let training = proof(
        stage,
        session,
        session_digest,
        0,
        AttemptRole::TrainingBaseline,
        session.initial_candidate_digest,
        Point {
            loss: 1.25,
            effort: 0.25,
            objective: 0.20,
        },
    );
    // The authorization is built first: a challenger's run identity is not
    // valid without it, so the proof cannot be assembled before it exists.
    let transition = transition_reference(session, session_digest, frozen_candidate);
    let challenger = super::proof_with_objectives(
        stage,
        session,
        session_digest,
        1,
        AttemptRole::TrainingChallenger { attempt_index: 0 },
        frozen_candidate,
        Point {
            loss: 0.90,
            effort: 0.30,
            objective: 0.20,
        },
        None,
        Some(transition.1),
    );
    (training, transition, challenger)
}

pub(super) fn transition_reference(
    session: &SessionIdentity,
    session_digest: Digest,
    frozen_candidate: Digest,
) -> (
    flight_tune::CandidateTransitionReceipt,
    CandidateTransitionReference,
) {
    let source = tuning_candidate(0.0);
    let target = tuning_candidate(0.5);
    let request = CandidateTransitionRequest::new(
        session_digest,
        &source,
        session.initial_candidate_digest,
        &target,
        frozen_candidate,
        session.runtimes.transition_validator.clone(),
        session.runtimes.adjacency_policy_digest,
        fixed_digest(61),
    )
    .expect("create fixture transition request");
    let receipt = flight_tune::CandidateTransitionReceipt::authorized(&request)
        .expect("authorize fixture transition");
    let reference = receipt.reference();
    (receipt, reference)
}
