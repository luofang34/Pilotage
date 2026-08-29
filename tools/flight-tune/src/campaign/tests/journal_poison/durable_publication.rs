mod storage;

use pilotage_durable_storage::{
    DurabilityStep, FaultAction, FaultController, FaultRule, StorageError, StorageOperation,
};

use super::evidence::ActionSnapshot;
use super::rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    candidate, stage,
};
use super::{
    Journal, JournalEvent, TestDirectory, TestTuner, Tuner, assert_ambiguous, assert_poisoned,
};
use crate::{AttemptRole, Digest, JournalEntry, TuneError};
use storage::{PublicationSnapshot, assert_head_matches, assert_publication_objects};

#[derive(Clone, Copy)]
enum PublicationFault {
    BeforeRename,
    AmbiguousAfterRename,
}

#[derive(Debug, PartialEq, Eq)]
struct MutationCounts {
    backend: [usize; 7],
    vehicle: [usize; 3],
    gates: [usize; 4],
    metric: [usize; 4],
}

#[test]
fn candidate_authorization_failure_leaves_an_inert_exact_entry() {
    check_candidate_authorization_publication(PublicationFault::BeforeRename);
}

#[test]
fn candidate_authorization_ambiguity_recovers_the_exact_entry() {
    check_candidate_authorization_publication(PublicationFault::AmbiguousAfterRename);
}

#[test]
fn run_preparation_failure_prevents_all_run_side_effects() {
    check_run_preparation_publication(PublicationFault::BeforeRename);
}

#[test]
fn run_preparation_ambiguity_prevents_all_run_side_effects() {
    check_run_preparation_publication(PublicationFault::AmbiguousAfterRename);
}

fn check_candidate_authorization_publication(fault: PublicationFault) {
    let directory = TestDirectory::new(&fault.label("candidate-authorization"));
    let state = FakeHandle::new();
    let mut tuner = open_tuner(&directory, state.clone(), vec![0.5]);
    tuner
        .run_training_attempts_blocking(0)
        .expect("complete the training baseline");
    let actions_before = MutationCounts::new(&state);
    let proposals_before = tuner.strategy.views.borrow().len();
    let authorizations_before = state.0.borrow().transition.authorization_count;
    let publication_before = PublicationSnapshot::new(&directory);
    let faults = fault.controller();
    let mut tuner = reopen_tuner_with_faults(tuner, &directory, faults.clone());

    let error = tuner
        .run_one_training_challenger_blocking()
        .expect_err("reject the authorization publication");

    fault.assert_error(error);
    assert!(faults.is_exhausted().expect("read fault state"));
    assert_eq!(MutationCounts::new(&state), actions_before);
    assert_eq!(tuner.strategy.views.borrow().len(), proposals_before + 1);
    assert_eq!(
        state.0.borrow().transition.authorization_count,
        authorizations_before + 1
    );
    let target = crate::campaign::evaluate::candidate_digest(&candidate(0.5))
        .expect("digest target candidate");
    let publication_after = PublicationSnapshot::new(&directory);
    let orphan = assert_publication_objects(
        &directory,
        &publication_before,
        &publication_after,
        Some(target),
    );
    let source = crate::campaign::evaluate::candidate_digest(&candidate(0.0))
        .expect("digest source candidate");
    assert_authorization_entry(&orphan, source, target);
    assert_eq!(authorization_count(tuner.journal()), 0);
    let poisoned_actions = MutationCounts::new(&state);
    assert_poisoned(tuner.freeze_candidate());
    assert_eq!(MutationCounts::new(&state), poisoned_actions);
    finish_candidate_recovery(
        fault,
        tuner,
        &directory,
        source,
        target,
        &publication_before,
    );
}

fn check_run_preparation_publication(fault: PublicationFault) {
    let directory = TestDirectory::new(&fault.label("run-preparation"));
    let state = FakeHandle::new();
    let mut tuner = open_tuner(&directory, state.clone(), Vec::new());
    let initial = candidate(0.0);
    let digest = crate::campaign::evaluate::candidate_digest(&initial).expect("digest candidate");
    let plan = AttemptRole::TrainingBaseline { suite_index: 0 }
        .plan_digest(&tuner.stage, digest, tuner.journal.session().fixed_seed)
        .expect("create run plan");
    let (trial_id, stored) = tuner
        .journal
        .prepare_attempt(
            AttemptRole::TrainingBaseline { suite_index: 0 },
            &initial,
            plan,
            None,
        )
        .expect("prepare the attempt");
    assert_eq!(stored, digest);
    let actions_before = ActionSnapshot::new(&state, &tuner.strategy.views);
    let contexts_before = transition_context_counts(&state);
    let publication_before = PublicationSnapshot::new(&directory);
    let faults = fault.controller();
    let mut tuner = reopen_tuner_with_faults(tuner, &directory, faults.clone());

    let error = run_prepared_baseline(&mut tuner, trial_id, &initial, digest)
        .expect_err("reject the run preparation publication");

    fault.assert_error(error);
    assert!(faults.is_exhausted().expect("read fault state"));
    assert_eq!(
        ActionSnapshot::new(&state, &tuner.strategy.views),
        actions_before
    );
    assert_eq!(transition_context_counts(&state), contexts_before);
    let publication_after = PublicationSnapshot::new(&directory);
    let orphan =
        assert_publication_objects(&directory, &publication_before, &publication_after, None);
    assert_run_prepared_entry(&orphan, trial_id, digest);
    assert_eq!(run_prepared_count(tuner.journal()), 0);
    assert_poisoned(tuner.freeze_candidate());
    assert_eq!(
        ActionSnapshot::new(&state, &tuner.strategy.views),
        actions_before
    );
    finish_run_recovery(
        fault,
        tuner,
        &directory,
        trial_id,
        digest,
        &publication_before,
    );
}

fn run_prepared_baseline(
    tuner: &mut TestTuner,
    trial_id: u64,
    initial: &crate::Candidate,
    digest: Digest,
) -> Result<(), TuneError> {
    crate::campaign::evaluate::run_prepared_blocking(
        &mut tuner.journal,
        &tuner.stage,
        trial_id,
        AttemptRole::TrainingBaseline { suite_index: 0 },
        initial,
        digest,
        None,
        &mut tuner.backend,
        &mut tuner.vehicle,
        &tuner.capability,
        &mut tuner.gates,
        &mut tuner.metric,
    )
}

fn open_tuner(directory: &TestDirectory, state: FakeHandle, values: Vec<f64>) -> TestTuner {
    Tuner::open_or_resume(
        directory.path(),
        stage(),
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::tracked(2.0, state.clone()),
        QuadraticMetric::new(state),
        SequenceStrategy::new(values),
    )
    .expect("open tuner")
}

fn reopen_tuner_with_faults(
    tuner: TestTuner,
    directory: &TestDirectory,
    faults: FaultController,
) -> TestTuner {
    let Tuner {
        stage,
        backend,
        vehicle,
        capability,
        gates,
        metric,
        strategy,
        journal,
    } = tuner;
    let runtimes = journal.session().runtimes.clone();
    let fixed_seed = journal.session().fixed_seed;
    let initial_digest = journal.session().initial_candidate_digest;
    let initial = journal
        .read_candidate(initial_digest)
        .expect("read initial candidate");
    drop(journal);
    let journal = Journal::open_or_create_with_faults(
        directory.path(),
        &stage,
        fixed_seed,
        runtimes,
        &initial,
        faults,
    )
    .expect("reopen journal with a publication fault");
    Tuner {
        stage,
        backend,
        vehicle,
        capability,
        gates,
        metric,
        strategy,
        journal,
    }
}

fn finish_candidate_recovery(
    fault: PublicationFault,
    tuner: TestTuner,
    directory: &TestDirectory,
    source: Digest,
    target: Digest,
    before: &PublicationSnapshot,
) {
    let journal = reopen_durable_journal(tuner, directory);
    match fault {
        PublicationFault::BeforeRename => {
            assert_eq!(authorization_count(&journal), 0);
            assert_eq!(PublicationSnapshot::new(directory).head, before.head);
        }
        PublicationFault::AmbiguousAfterRename => {
            assert_eq!(authorization_count(&journal), 1);
            let entry = journal.entries().last().expect("authorization entry");
            assert_authorization_entry(entry, source, target);
            assert_head_matches(directory, entry);
        }
    }
}

fn finish_run_recovery(
    fault: PublicationFault,
    tuner: TestTuner,
    directory: &TestDirectory,
    trial_id: u64,
    candidate_digest: Digest,
    before: &PublicationSnapshot,
) {
    let journal = reopen_durable_journal(tuner, directory);
    match fault {
        PublicationFault::BeforeRename => {
            assert_eq!(run_prepared_count(&journal), 0);
            assert_eq!(PublicationSnapshot::new(directory).head, before.head);
        }
        PublicationFault::AmbiguousAfterRename => {
            assert_eq!(run_prepared_count(&journal), 1);
            let entry = journal.entries().last().expect("run preparation entry");
            assert_run_prepared_entry(entry, trial_id, candidate_digest);
            assert_head_matches(directory, entry);
        }
    }
}

fn reopen_durable_journal(tuner: TestTuner, directory: &TestDirectory) -> Journal {
    let runtimes = tuner.journal.session().runtimes.clone();
    let fixed_seed = tuner.journal.session().fixed_seed;
    let campaign_stage = tuner.stage.clone();
    let initial = candidate(0.0);
    drop(tuner);
    Journal::open_or_create(
        directory.path(),
        &campaign_stage,
        fixed_seed,
        runtimes,
        &initial,
    )
    .expect("reopen durable journal")
}

impl MutationCounts {
    fn new(state: &FakeHandle) -> Self {
        let state = state.0.borrow();
        Self {
            backend: [
                state.open_session_count,
                state.prepare_count,
                state.start_count,
                state.sample_poll_count,
                state.sample_count,
                state.stop_count,
                state.cleanup_count,
            ],
            vehicle: [
                state.vehicle.bind_count,
                state.vehicle.ensure_count,
                state.vehicle.apply_count,
            ],
            gates: [
                state.gate_begin_count,
                state.gate_evaluate_count,
                state.gate_finish_count,
                state.gate_cancel_count,
            ],
            metric: [
                state.metric_begin_count,
                state.metric_observe_count,
                state.metric_finish_count,
                state.metric_cancel_count,
            ],
        }
    }
}

impl PublicationFault {
    fn label(self, event: &str) -> String {
        let mode = match self {
            Self::BeforeRename => "failure",
            Self::AmbiguousAfterRename => "ambiguity",
        };
        format!("{event}-{mode}")
    }

    fn controller(self) -> FaultController {
        match self {
            Self::BeforeRename => FaultController::new([FaultRule::once(
                StorageOperation::CompareExchange,
                DurabilityStep::AuthorizationRename,
                FaultAction::FailBefore,
            )]),
            Self::AmbiguousAfterRename => FaultController::new([
                FaultRule::once(
                    StorageOperation::CompareExchange,
                    DurabilityStep::ParentDirectory,
                    FaultAction::LoseAckAfter,
                ),
                FaultRule::once(
                    StorageOperation::CompareExchange,
                    DurabilityStep::RecoveryBarrier,
                    FaultAction::LoseAckAfter,
                ),
            ]),
        }
    }

    fn assert_error(self, error: TuneError) {
        match self {
            Self::BeforeRename => assert!(matches!(
                error,
                TuneError::Storage { source }
                    if matches!(source.as_ref(), StorageError::InjectedFault { context }
                        if context.operation == StorageOperation::CompareExchange
                            && context.step == DurabilityStep::AuthorizationRename)
            )),
            Self::AmbiguousAfterRename => assert_ambiguous(error),
        }
    }
}

fn transition_context_counts(state: &FakeHandle) -> [usize; 3] {
    let state = state.0.borrow();
    [
        state.transition.prepared_contexts.len(),
        state.transition.started_contexts.len(),
        state.transition.vehicle_contexts.len(),
    ]
}

fn authorization_count(journal: &Journal) -> usize {
    journal
        .entries()
        .iter()
        .filter(|entry| {
            matches!(
                entry.event,
                JournalEvent::CandidateTransitionAuthorized { .. }
            )
        })
        .count()
}

fn run_prepared_count(journal: &Journal) -> usize {
    journal
        .entries()
        .iter()
        .filter(|entry| matches!(entry.event, JournalEvent::RunPrepared { .. }))
        .count()
}

fn assert_authorization_entry(entry: &JournalEntry, source: Digest, target: Digest) {
    let JournalEvent::CandidateTransitionAuthorized {
        candidate, receipt, ..
    } = &entry.event
    else {
        panic!("expected a candidate transition authorization");
    };
    assert_eq!(*candidate, target);
    assert_eq!(receipt.source_candidate_digest(), source);
    assert_eq!(receipt.target_candidate_digest(), target);
    let reference = receipt.reference();
    assert_eq!(reference.source_candidate_digest(), source);
    assert_eq!(reference.target_candidate_digest(), target);
    assert!(reference.is_valid_for_target(target));
}

fn assert_run_prepared_entry(entry: &JournalEntry, trial_id: u64, candidate_digest: Digest) {
    let JournalEvent::RunPrepared {
        trial_id: saved_trial,
        context,
        run_intent_digest,
        ..
    } = &entry.event
    else {
        panic!("expected a run preparation");
    };
    assert_eq!(*saved_trial, trial_id);
    assert_eq!(context.trial_id(), trial_id);
    assert_eq!(
        context.role(),
        AttemptRole::TrainingBaseline { suite_index: 0 }
    );
    assert_eq!(context.candidate_digest(), candidate_digest);
    assert_eq!(context.transition_authorization(), None);
    assert_eq!(
        *run_intent_digest,
        context.digest().expect("digest run context")
    );
}
