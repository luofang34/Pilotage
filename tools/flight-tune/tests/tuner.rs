//! End-to-end fault tests for the isolated tuning campaign.

#![allow(clippy::expect_used, clippy::panic)]

#[path = "tuner/journal_storage.rs"]
mod journal_storage;
#[path = "tuner/no_samples.rs"]
mod no_samples;
#[path = "tuner/promotion_chain_tamper.rs"]
mod promotion_chain_tamper;
#[path = "tuner/promotion_evidence_tamper.rs"]
mod promotion_evidence_tamper;
#[path = "tuner/promotion_recovery.rs"]
mod promotion_recovery;
#[path = "tuner/promotion_snapshot_authority.rs"]
mod promotion_snapshot_authority;
#[path = "tuner/reconciliation.rs"]
mod reconciliation;
#[path = "tuner/recovery_order.rs"]
mod recovery_order;
#[path = "tuner/terminal.rs"]
mod terminal;
#[path = "tuner/test_rig.rs"]
mod test_rig;
#[path = "tuner/transition_authorization.rs"]
mod transition_authorization;
#[path = "tuner/transition_chain_tamper.rs"]
mod transition_chain_tamper;

use std::path::Path;

use flight_tune::{
    CampaignPhase, CandidateEvaluation, FinalQualificationOutcome, JournalEvent, PromotionDecision,
    TuneError, Tuner,
};
use test_rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, FakeVehicle, QuadraticMetric,
    SequenceStrategy, TestDirectory, assert_receipt_error, candidate, stage,
};

type TestTuner = Tuner<FakeBackend, FakeVehicle, EnvelopeGates, QuadraticMetric, SequenceStrategy>;

#[test]
fn training_cannot_observe_hidden_sets_and_the_campaign_seals_once() {
    let directory = TestDirectory::new("isolated-flow");
    let state = FakeHandle::new();
    let strategy = SequenceStrategy::new(vec![0.5]);
    let views = strategy.views.clone();
    let mut tuner = open(directory.path(), state.clone(), strategy, 2.0).expect("open tuner");

    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");

    assert_eq!(views.borrow().len(), 1);
    assert_eq!(views.borrow()[0].0, vec!["training-calm"]);
    assert!(views.borrow()[0].1.is_empty());
    assert!(
        state
            .0
            .borrow()
            .scenario_runs
            .iter()
            .all(|(id, _, _)| id == "training-calm")
    );

    tuner.freeze_candidate().expect("freeze candidate");
    assert!(tuner.journal().verified_evidence_snapshot().is_err());
    let promotion = tuner.run_promotion_once_blocking().expect("run promotion");
    assert!(matches!(promotion, PromotionDecision::Promoted { .. }));
    let promotion_evidence = tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("verify promotion evidence");
    assert_eq!(promotion_evidence.promotion_closure.decision, promotion);
    assert!(promotion_evidence.promotion_frozen.is_some());
    assert!(promotion_evidence.final_proof.is_none());
    assert_eq!(state.0.borrow().vehicle.gain, 0.5);
    let runs_after_promotion = state.0.borrow().scenario_runs.len();
    assert_eq!(
        tuner.run_promotion_once_blocking().expect("read promotion"),
        promotion
    );
    assert_eq!(state.0.borrow().scenario_runs.len(), runs_after_promotion);
    promotion_snapshot_authority::assert_promotion_uses_paired_seeds(&state);

    let final_outcome = tuner
        .run_final_qualification_once_blocking()
        .expect("run final qualification");
    assert_eq!(final_outcome, FinalQualificationOutcome::Qualified);
    let runs_after_final = state.0.borrow().scenario_runs.len();
    assert_eq!(
        tuner
            .run_final_qualification_once_blocking()
            .expect("read final result"),
        final_outcome
    );
    assert_eq!(state.0.borrow().scenario_runs.len(), runs_after_final);
    assert_eq!(tuner.journal().phase(), CampaignPhase::Sealed);
    let sealed_evidence = tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("verify sealed evidence");
    assert!(sealed_evidence.final_proof.is_some());
    assert_eq!(sealed_evidence.final_outcome, Some(final_outcome));
    assert_eq!(
        tuner.qualified_candidate().expect("qualified candidate"),
        candidate(0.5)
    );
    assert_eq!(state.0.borrow().vehicle.gain, 0.5);
    assert!(matches!(
        tuner.run_training_attempts_blocking(1),
        Err(TuneError::InvalidState { .. })
    ));
}

#[test]
fn promotion_rejects_an_improvement_below_the_relative_floor() {
    let directory = TestDirectory::new("relative-promotion-floor");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.1]),
        2.0,
    )
    .expect("open tuner");

    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");

    assert!(matches!(
        tuner.run_promotion_once_blocking().expect("run promotion"),
        PromotionDecision::RejectedNoImprovement { .. }
    ));
    assert!(matches!(
        tuner
            .run_final_qualification_once_blocking()
            .expect("run final qualification"),
        FinalQualificationOutcome::FailedObjective { .. }
    ));
}

#[test]
fn final_qualification_rejects_a_named_objective_limit() {
    let directory = TestDirectory::new("named-final-objective");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![1.0]),
        2.0,
    )
    .expect("open tuner");

    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    assert!(matches!(
        tuner.run_promotion_once_blocking().expect("run promotion"),
        PromotionDecision::Promoted { .. }
    ));
    assert_eq!(
        tuner
            .run_final_qualification_once_blocking()
            .expect("run final qualification"),
        FinalQualificationOutcome::FailedObjective {
            metric: "test.response".to_owned(),
        }
    );
    assert_eq!(state.0.borrow().vehicle.gain, 0.0);
}

#[test]
fn final_qualification_rejects_a_missing_named_objective() {
    let directory = TestDirectory::new("missing-final-objective");
    let state = FakeHandle::new();
    let mut policy = stage();
    policy
        .qualification
        .objective_maxima
        .insert("required.missing".to_owned(), 1.0);
    let mut tuner = open_stage(
        directory.path(),
        state,
        SequenceStrategy::new(vec![0.5]),
        2.0,
        policy,
    )
    .expect("open tuner");

    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    assert!(matches!(
        tuner.run_promotion_once_blocking().expect("run promotion"),
        PromotionDecision::Promoted { .. }
    ));
    assert_eq!(
        tuner
            .run_final_qualification_once_blocking()
            .expect("run final qualification"),
        FinalQualificationOutcome::FailedObjective {
            metric: "required.missing".to_owned(),
        }
    );
}

#[test]
fn a_streaming_gate_stops_before_metric_scoring_and_saves_failure_first() {
    let directory = TestDirectory::new("stream-gate");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![1.5]),
        1.0,
    )
    .expect("open tuner");

    tuner
        .run_training_attempts_blocking(1)
        .expect("run hard gate challenger");

    let state_ref = state.0.borrow();
    assert_eq!(state_ref.sample_count, 3);
    assert_eq!(state_ref.metric_observe_count, 2);
    assert_eq!(state_ref.stop_count, 3);
    assert_eq!(state_ref.cleanup_count, 3);
    drop(state_ref);
    let failure_index = tuner
        .journal()
        .entries()
        .iter()
        .position(|entry| {
            matches!(
                entry.event,
                JournalEvent::AttemptCompleted {
                    evaluation: CandidateEvaluation::HardGateFailed { .. },
                    ..
                }
            )
        })
        .expect("hard gate event");
    let cleanup_index = tuner
        .journal()
        .entries()
        .iter()
        .enumerate()
        .skip(failure_index.wrapping_add(1))
        .find_map(|(index, entry)| {
            matches!(entry.event, JournalEvent::CleanupRecorded { .. }).then_some(index)
        })
        .expect("cleanup event");
    assert!(failure_index < cleanup_index);
}

#[test]
fn recovery_quarantines_a_prepared_candidate_without_replaying_it() {
    let directory = TestDirectory::new("crash-recovery");
    let state = FakeHandle::new();
    state.0.borrow_mut().panic_on_prepare = Some(3);
    let strategy = SequenceStrategy::new(vec![0.5, 0.75]);
    let mut tuner =
        open(directory.path(), state.clone(), strategy.clone(), 2.0).expect("open tuner");

    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tuner.run_training_attempts_blocking(1).ok();
    }));
    assert!(stopped.is_err());
    drop(tuner);
    assert_eq!(state.0.borrow().vehicle.apply_count, 1);
    state.0.borrow_mut().panic_on_prepare = None;

    let mut resumed = open(directory.path(), state.clone(), strategy, 2.0).expect("resume tuner");

    assert_eq!(resumed.journal().training_attempt_count(), 1);
    assert_eq!(state.0.borrow().vehicle.apply_count, 1);
    assert!(
        resumed
            .journal()
            .entries()
            .iter()
            .any(|entry| matches!(entry.event, JournalEvent::AttemptQuarantined { .. }))
    );
    resumed
        .run_training_attempts_blocking(1)
        .expect("run next candidate");
    assert!(
        state
            .0
            .borrow()
            .scenario_runs
            .iter()
            .all(|(_, _, gain)| *gain != 0.5)
    );
}

#[test]
fn candidate_readback_mismatch_is_quarantined_before_cleanup() {
    let directory = TestDirectory::new("readback");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open tuner");
    state
        .0
        .borrow_mut()
        .vehicle
        .bad_candidate_readback_on_ensure = Some(2);

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject readback");

    assert_receipt_error(error);
    assert_eq!(state.0.borrow().start_count, 0);
    assert_eq!(state.0.borrow().cleanup_count, 1);
    let events = tuner.journal().entries();
    let quarantine = events
        .iter()
        .position(|entry| matches!(entry.event, JournalEvent::AttemptQuarantined { .. }))
        .expect("quarantine event");
    let cleanup = events
        .iter()
        .position(|entry| matches!(entry.event, JournalEvent::CleanupRecorded { .. }))
        .expect("cleanup event");
    assert!(quarantine < cleanup);
}

#[test]
fn scenario_readback_mismatch_stops_and_quarantines_the_attempt() {
    let directory = TestDirectory::new("scenario-readback");
    let state = FakeHandle::new();
    state.0.borrow_mut().bad_scenario_readback = true;
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open tuner");

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject scenario readback");

    assert_receipt_error(error);
    assert_eq!(state.0.borrow().sample_count, 0);
    assert_eq!(state.0.borrow().stop_count, 1);
    assert_eq!(state.0.borrow().cleanup_count, 1);
    let events = tuner.journal().entries();
    let quarantine = events
        .iter()
        .position(|entry| matches!(entry.event, JournalEvent::AttemptQuarantined { .. }))
        .expect("quarantine event");
    let cleanup = events
        .iter()
        .position(|entry| matches!(entry.event, JournalEvent::CleanupRecorded { .. }))
        .expect("cleanup event");
    assert!(quarantine < cleanup);
}

#[test]
fn a_hardware_like_factory_cannot_get_a_simulator_binding() {
    let directory = TestDirectory::new("hardware-binding");
    let state = FakeHandle::new();
    let result = Tuner::open_or_resume(
        directory.path(),
        stage(),
        4,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::hardware_like(state.clone()),
        EnvelopeGates::new(2.0),
        QuadraticMetric::new(state.clone()),
        SequenceStrategy::new(Vec::new()),
    );

    assert!(matches!(result, Err(TuneError::Adapter { .. })));
    assert_eq!(state.0.borrow().vehicle.apply_count, 0);
}

#[test]
fn changed_strategy_identity_cannot_resume_the_session() {
    let directory = TestDirectory::new("identity-mismatch");
    let state = FakeHandle::new();
    let tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.5]),
        2.0,
    )
    .expect("open tuner");
    drop(tuner);
    let apply_count = state.0.borrow().vehicle.apply_count;

    let result = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.75]),
        2.0,
    );

    assert!(matches!(result, Err(TuneError::JournalSessionMismatch)));
    assert_eq!(state.0.borrow().vehicle.apply_count, apply_count);
}

#[test]
fn a_second_writer_cannot_open_the_same_journal() {
    let directory = TestDirectory::new("writer-lock");
    let first_state = FakeHandle::new();
    let _first = open(
        directory.path(),
        first_state,
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open first writer");
    let second_state = FakeHandle::new();

    let second = open(
        directory.path(),
        second_state,
        SequenceStrategy::new(Vec::new()),
        2.0,
    );

    assert!(matches!(second, Err(TuneError::JournalLocked { .. })));
}

#[test]
fn sample_timeout_is_a_hard_gate_and_still_cleans_the_backend() {
    let directory = TestDirectory::new("timeout");
    let state = FakeHandle::new();
    state.0.borrow_mut().timeout_next_sample = true;
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open tuner");

    let result = tuner.run_training_attempts_blocking(0);

    assert!(matches!(result, Err(TuneError::UnsafeBaseline { .. })));
    assert_eq!(state.0.borrow().sample_count, 0);
    assert_eq!(state.0.borrow().metric_observe_count, 0);
    assert_eq!(state.0.borrow().stop_count, 1);
    assert_eq!(state.0.borrow().cleanup_count, 1);
}
fn open(
    path: &Path,
    state: FakeHandle,
    strategy: SequenceStrategy,
    gate_limit: f64,
) -> Result<TestTuner, TuneError> {
    open_stage(path, state, strategy, gate_limit, stage())
}

fn open_stage(
    path: &Path,
    state: FakeHandle,
    strategy: SequenceStrategy,
    gate_limit: f64,
    stage: flight_tune::SearchStage,
) -> Result<TestTuner, TuneError> {
    Tuner::open_or_resume(
        path,
        stage,
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::new(gate_limit),
        QuadraticMetric::new(state),
        strategy,
    )
}
