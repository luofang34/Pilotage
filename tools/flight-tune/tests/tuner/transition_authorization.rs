use std::fs;

use flight_tune::{AttemptRole, Digest, JournalEntry, JournalEvent, TuneError, Tuner};
use serde::Serialize;
use sha2::{Digest as ShaDigest, Sha256};

use super::test_rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    TestDirectory, candidate, stage,
};

#[path = "transition_authorization/durable_tree.rs"]
mod durable_tree;
#[path = "transition_authorization/intent_receipts.rs"]
mod intent_receipts;

use durable_tree::DurableTreeSnapshot;

#[test]
fn a_non_aviate_adapter_authorizes_each_training_transition() {
    let directory = TestDirectory::new("generic-transition-adapter");
    let state = FakeHandle::new();
    let mut tuner = open(
        &directory,
        state.clone(),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5, 0.75]),
    )
    .expect("open tuner");

    tuner
        .run_training_attempts_blocking(2)
        .expect("run two transitions");

    assert_eq!(state.0.borrow().transition.authorization_count, 2);
    assert_eq!(
        state.0.borrow().transition.checks,
        vec![(0.0, 0.5), (0.5, 0.75)]
    );
    assert_eq!(transition_entries(&tuner).count(), 2);
}

#[test]
fn later_incumbent_rejection_has_no_external_side_effect() {
    let directory = TestDirectory::new("later-incumbent-adjacency");
    let state = FakeHandle::new();
    state.0.borrow_mut().transition.maximum_delta = Some(0.25);
    let mut tuner = Tuner::open_or_resume(
        directory.path(),
        stage(),
        91,
        candidate(0.5),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::tracked(2.0, state.clone()),
        QuadraticMetric::new(state.clone()),
        SequenceStrategy::new(vec![0.7, 0.3]),
    )
    .expect("open tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("accept adjacent transition");
    let before = ExternalMutations::capture(&state);
    let journal_length = tuner.journal().entries().len();
    let durable_before = DurableTreeSnapshot::capture(directory.path());

    let error = tuner
        .run_training_attempts_blocking(1)
        .expect_err("reject nonadjacent transition");

    assert!(matches!(error, TuneError::Adapter { .. }));
    assert_eq!(ExternalMutations::capture(&state), before);
    assert_eq!(tuner.journal().entries().len(), journal_length);
    durable_before.assert_unchanged(directory.path());
    assert_eq!(
        state.0.borrow().transition.checks,
        vec![(0.5, 0.7), (0.7, 0.3)]
    );
}

#[test]
fn one_transition_reference_closes_the_execution_chain() {
    let directory = TestDirectory::new("transition-execution-chain");
    let state = FakeHandle::new();
    let mut tuner = open(
        &directory,
        state.clone(),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("open tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run challenger");

    let entries = tuner.journal().entries();
    let (authorization_index, reference) = entries
        .iter()
        .enumerate()
        .find_map(|(index, entry)| match &entry.event {
            JournalEvent::CandidateTransitionAuthorized { receipt, .. } => {
                Some((index, receipt.reference()))
            }
            _ => None,
        })
        .expect("transition authorization");
    let (attempt_index, candidate_digest) = entries
        .iter()
        .enumerate()
        .find_map(|(index, entry)| match &entry.event {
            JournalEvent::AttemptPrepared {
                role: AttemptRole::TrainingChallenger { .. },
                candidate,
                transition,
                ..
            } => {
                assert_eq!(*transition, Some(reference));
                Some((index, *candidate))
            }
            _ => None,
        })
        .expect("challenger attempt");
    let runs = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| match &entry.event {
            JournalEvent::RunPrepared {
                context,
                run_intent_digest,
                ..
            } if matches!(context.role(), AttemptRole::TrainingChallenger { .. }) => {
                Some((index, context, *run_intent_digest))
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(!runs.is_empty());
    assert!(authorization_index < attempt_index);
    let durable_contexts = entries
        .iter()
        .filter_map(|entry| match &entry.event {
            JournalEvent::RunPrepared { context, .. } => Some(context.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for (run_index, context, run_intent_digest) in runs {
        assert!(attempt_index < run_index);
        assert_eq!(context.candidate_digest(), candidate_digest);
        assert_eq!(context.transition_authorization(), Some(reference));
        assert_eq!(context.digest().expect("context digest"), run_intent_digest);
    }
    assert_observed_contexts(&state, &durable_contexts);
}

#[test]
fn resume_consumes_the_exact_saved_target_without_reproposal() {
    let directory = TestDirectory::new("resume-saved-transition");
    let state = FakeHandle::new();
    let strategy = SequenceStrategy::new(vec![0.5]);
    let views = strategy.views.clone();
    let mut tuner = open(
        &directory,
        state.clone(),
        FakeFactory::new(state.clone()),
        strategy.clone(),
    )
    .expect("open tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("create complete transition");
    let authorization = tuner
        .journal()
        .entries()
        .iter()
        .find(|entry| {
            matches!(
                entry.event,
                JournalEvent::CandidateTransitionAuthorized { .. }
            )
        })
        .cloned()
        .expect("saved transition authorization");
    let reference = match &authorization.event {
        JournalEvent::CandidateTransitionAuthorized { receipt, .. } => receipt.reference(),
        _ => panic!("saved event is not an authorization"),
    };
    let authorization_digest = document_digest(&authorization);
    drop(tuner);
    write_head(directory.path(), authorization_digest);
    let proposals_before_resume = views.borrow().len();

    let mut resumed = open(
        &directory,
        state.clone(),
        FakeFactory::new(state.clone()),
        strategy,
    )
    .expect("resume tuner");
    resumed
        .run_training_attempts_blocking(1)
        .expect("consume saved transition");

    assert_eq!(views.borrow().len(), proposals_before_resume);
    assert_eq!(
        resumed
            .journal()
            .training_incumbent()
            .expect("training incumbent"),
        candidate(0.5)
    );
    assert_eq!(state.0.borrow().transition.checks.len(), 2);
    assert_resumed_reference(&resumed, reference);
}

fn assert_resumed_reference(
    tuner: &super::TestTuner,
    reference: flight_tune::CandidateTransitionReference,
) {
    let entries = tuner.journal().entries();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.event,
                    JournalEvent::CandidateTransitionAuthorized { .. }
                )
            })
            .count(),
        1
    );
    let attempt_reference = entries.iter().find_map(|entry| match &entry.event {
        JournalEvent::AttemptPrepared {
            role: AttemptRole::TrainingChallenger { .. },
            transition,
            ..
        } => *transition,
        _ => None,
    });
    assert_eq!(attempt_reference, Some(reference));
    let run_references = entries
        .iter()
        .filter_map(|entry| match &entry.event {
            JournalEvent::RunPrepared { context, .. }
                if matches!(context.role(), AttemptRole::TrainingChallenger { .. }) =>
            {
                context.transition_authorization()
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!run_references.is_empty());
    assert!(run_references.iter().all(|saved| *saved == reference));
}

#[test]
fn changed_validator_or_policy_cannot_resume() {
    for changed_factory in [
        FakeFactory::with_transition_validator as fn(FakeHandle, &str) -> FakeFactory,
        FakeFactory::with_adjacency_policy,
    ] {
        let directory = TestDirectory::new("changed-transition-runtime");
        let state = FakeHandle::new();
        let tuner = open(
            &directory,
            state.clone(),
            FakeFactory::new(state.clone()),
            SequenceStrategy::new(Vec::new()),
        )
        .expect("open tuner");
        drop(tuner);
        let before = OpenMutations::capture(&state);

        let result = open(
            &directory,
            state.clone(),
            changed_factory(state.clone(), "changed-transition-runtime"),
            SequenceStrategy::new(Vec::new()),
        );

        assert!(matches!(result, Err(TuneError::JournalSessionMismatch)));
        assert_eq!(OpenMutations::capture(&state), before);
    }
}

fn open(
    directory: &TestDirectory,
    state: FakeHandle,
    factory: FakeFactory,
    strategy: SequenceStrategy,
) -> Result<super::TestTuner, TuneError> {
    Tuner::open_or_resume(
        directory.path(),
        stage(),
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        factory,
        EnvelopeGates::tracked(2.0, state.clone()),
        QuadraticMetric::new(state),
        strategy,
    )
}

fn transition_entries(tuner: &super::TestTuner) -> impl Iterator<Item = &JournalEntry> {
    tuner.journal().entries().iter().filter(|entry| {
        matches!(
            entry.event,
            JournalEvent::CandidateTransitionAuthorized { .. }
        )
    })
}

fn assert_observed_contexts(state: &FakeHandle, durable: &[flight_tune::RunExecutionContext]) {
    let observed = &state.0.borrow().transition;
    assert_eq!(observed.prepared_contexts, durable);
    assert_eq!(observed.vehicle_contexts, durable);
    assert_eq!(observed.started_contexts, durable);
}

fn document_digest(value: &impl Serialize) -> Digest {
    let bytes = serde_json::to_vec(value).expect("encode journal entry");
    Digest::from_bytes(Sha256::digest(bytes).into())
}

fn write_head(root: &std::path::Path, digest: Digest) {
    let bytes =
        serde_json::to_vec(&serde_json::json!({ "digest": digest })).expect("encode journal head");
    fs::write(root.join("HEAD.json"), bytes).expect("rewind journal head");
}

#[derive(Debug, PartialEq, Eq)]
struct ExternalMutations {
    prepare: usize,
    ensure: usize,
    apply: usize,
    start: usize,
    sample_poll: usize,
    stop: usize,
    cleanup: usize,
    gate_begin: usize,
    gate_evaluate: usize,
    gate_finish: usize,
    gate_cancel: usize,
    metric_begin: usize,
    metric_observe: usize,
    metric_finish: usize,
    metric_cancel: usize,
}

impl ExternalMutations {
    fn capture(handle: &FakeHandle) -> Self {
        let state = handle.0.borrow();
        Self {
            prepare: state.prepare_count,
            ensure: state.vehicle.ensure_count,
            apply: state.vehicle.apply_count,
            start: state.start_count,
            sample_poll: state.sample_poll_count,
            stop: state.stop_count,
            cleanup: state.cleanup_count,
            gate_begin: state.gate_begin_count,
            gate_evaluate: state.gate_evaluate_count,
            gate_finish: state.gate_finish_count,
            gate_cancel: state.gate_cancel_count,
            metric_begin: state.metric_begin_count,
            metric_observe: state.metric_observe_count,
            metric_finish: state.metric_finish_count,
            metric_cancel: state.metric_cancel_count,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct OpenMutations {
    open_session: usize,
    bind: usize,
    authorize_transition: usize,
    external: ExternalMutations,
}

impl OpenMutations {
    fn capture(handle: &FakeHandle) -> Self {
        let state = handle.0.borrow();
        let open_session = state.open_session_count;
        let bind = state.vehicle.bind_count;
        let authorize_transition = state.transition.authorization_count;
        drop(state);
        Self {
            open_session,
            bind,
            authorize_transition,
            external: ExternalMutations::capture(handle),
        }
    }
}
