use std::fs;

use flight_tune::{
    AttemptRole, ControlFamily, Digest, JournalEntry, JournalEvent, SearchStage,
    SimulatorVehicleFactory, TuneError, Tuner, scenario_runtime_identity,
};
use serde::Deserialize;

use super::TestTuner;
use super::test_rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    TestDirectory, candidate, stage, stage_with_changed_suite, stage_with_changed_training_mission,
    stage_with_stimulus_family,
};

#[test]
fn changed_runtime_orphans_baseline_and_pending_challenger_before_mutation() {
    let old_directory = TestDirectory::new("orphan-baseline-old-runtime");
    let state = FakeHandle::new();
    state.0.borrow_mut().panic_on_prepare = Some(3);
    let strategy = SequenceStrategy::new(vec![0.5]);
    let mut tuner = open_with_factory(
        &old_directory,
        state.clone(),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        strategy.clone(),
    )
    .expect("open old runtime");
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tuner.run_training_attempts_blocking(1).ok();
    }));
    assert!(stopped.is_err());
    drop(tuner);
    state.0.borrow_mut().panic_on_prepare = None;

    let old_head = read_head_entry(&old_directory);
    assert!(read_entries(&old_directory).iter().any(|entry| matches!(
        &entry.event,
        JournalEvent::AttemptPrepared {
            role: AttemptRole::TrainingChallenger { .. },
            ..
        }
    )));
    let old_runtime = old_head
        .session
        .runtimes
        .scenario_runtime
        .clone()
        .expect("old scenario runtime identity");
    let before = ExternalMutations::capture(&state);
    let changed_port =
        FakeFactory::with_action_port_identity(state.clone(), "aviate-action-port-v2");
    let changed_runtime = scenario_runtime_identity(changed_port.scenario_action_port_identity())
        .expect("changed runtime identity");
    assert_ne!(changed_runtime, old_runtime);

    let result = open_with_factory(
        &old_directory,
        state.clone(),
        FakeBackend::with_action_port_identity(state.clone(), "aviate-action-port-v2"),
        changed_port,
        strategy,
    );

    assert!(matches!(result, Err(TuneError::JournalSessionMismatch)));
    assert_eq!(ExternalMutations::capture(&state), before);
    assert_eq!(read_head_entry(&old_directory), old_head);

    let new_directory = TestDirectory::new("orphan-baseline-new-runtime");
    let runs_before = state.0.borrow().scenario_runs.len();
    let mut new_tuner = open_with_factory(
        &new_directory,
        state.clone(),
        FakeBackend::with_action_port_identity(state.clone(), "aviate-action-port-v2"),
        FakeFactory::with_action_port_identity(state.clone(), "aviate-action-port-v2"),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("start exact-runtime session");
    new_tuner
        .run_training_attempts_blocking(1)
        .expect("run exact-runtime baseline and challenger");
    assert!(state.0.borrow().scenario_runs.len() > runs_before);
    assert_eq!(new_tuner.journal().training_attempt_count(), 1);
}

#[test]
fn changed_mission_document_orphans_baseline_before_mutation() {
    let directory = TestDirectory::new("orphan-baseline-old-mission");
    let state = FakeHandle::new();
    let mut tuner = open_with_stage(
        &directory,
        state.clone(),
        stage(),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("open old mission session");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run old mission training");
    drop(tuner);

    let old_head = read_head_entry(&directory);
    let before = ExternalMutations::capture(&state);
    let changed = stage_with_changed_training_mission();
    assert_ne!(
        changed.training_scenarios[0].content_digest,
        stage().training_scenarios[0].content_digest
    );

    let result = open_with_stage(
        &directory,
        state.clone(),
        changed.clone(),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    );

    assert!(matches!(result, Err(TuneError::JournalSessionMismatch)));
    assert_eq!(ExternalMutations::capture(&state), before);
    assert_eq!(read_head_entry(&directory), old_head);

    let new_directory = TestDirectory::new("orphan-baseline-new-mission");
    let runs_before = state.0.borrow().scenario_runs.len();
    let mut new_tuner = open_with_stage(
        &new_directory,
        state.clone(),
        changed,
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("start changed mission session");
    new_tuner
        .run_training_attempts_blocking(1)
        .expect("run changed mission baseline and challenger");
    assert!(state.0.borrow().scenario_runs.len() > runs_before);
    assert_eq!(new_tuner.journal().training_attempt_count(), 1);
}

#[test]
fn changed_stimulus_control_family_orphans_baseline_before_mutation() {
    let directory = TestDirectory::new("orphan-baseline-old-family");
    let state = FakeHandle::new();
    let operator = stage_with_stimulus_family(ControlFamily::OperatorVelocity);
    let direct = stage_with_stimulus_family(ControlFamily::DirectAttitudeThrust);
    assert_ne!(
        operator.training_scenarios[0].content_digest,
        direct.training_scenarios[0].content_digest
    );
    let mut tuner = open_with_stage(
        &directory,
        state.clone(),
        operator,
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("open operator-family session");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run operator-family training");
    drop(tuner);

    let old_head = read_head_entry(&directory);
    let before = ExternalMutations::capture(&state);
    let result = open_with_stage(
        &directory,
        state.clone(),
        direct.clone(),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    );

    assert!(matches!(result, Err(TuneError::JournalSessionMismatch)));
    assert_eq!(ExternalMutations::capture(&state), before);
    assert_eq!(read_head_entry(&directory), old_head);

    let new_directory = TestDirectory::new("orphan-baseline-new-family");
    let runs_before = state.0.borrow().scenario_runs.len();
    let mut new_tuner = open_with_stage(
        &new_directory,
        state.clone(),
        direct,
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("start direct-family session");
    new_tuner
        .run_training_attempts_blocking(1)
        .expect("run direct-family baseline and challenger");
    assert!(state.0.borrow().scenario_runs.len() > runs_before);
    assert_eq!(new_tuner.journal().training_attempt_count(), 1);
}

/// A frozen suite that changed orphans every result recorded under it.
///
/// The suite states which missions answer a search group and how many times
/// each one runs. A result produced under one suite cannot answer a decision
/// stated under another, so the campaign refuses to continue rather than
/// compare across the change.
#[test]
fn changed_training_suite_orphans_baseline_before_mutation() {
    let directory = TestDirectory::new("orphan-baseline-old-suite");
    let state = FakeHandle::new();
    let declared = stage();
    let changed = stage_with_changed_suite();
    assert_eq!(
        declared.training_scenarios, changed.training_scenarios,
        "only the suite declaration moved"
    );
    assert_ne!(
        declared.training_suites[0]
            .digest()
            .expect("the declared suite digest"),
        changed.training_suites[0]
            .digest()
            .expect("the changed suite digest")
    );
    let mut tuner = open_with_stage(
        &directory,
        state.clone(),
        declared,
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("open the declared suite session");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run the declared suite training");
    drop(tuner);

    let old_head = read_head_entry(&directory);
    let before = ExternalMutations::capture(&state);
    let result = open_with_stage(
        &directory,
        state.clone(),
        changed.clone(),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    );

    assert!(matches!(result, Err(TuneError::JournalSessionMismatch)));
    assert_eq!(ExternalMutations::capture(&state), before);
    assert_eq!(read_head_entry(&directory), old_head);

    let new_directory = TestDirectory::new("orphan-baseline-new-suite");
    let runs_before = state.0.borrow().scenario_runs.len();
    let mut new_tuner = open_with_stage(
        &new_directory,
        state.clone(),
        changed,
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("start the changed suite session");
    new_tuner
        .run_training_attempts_blocking(1)
        .expect("run the changed suite baseline and challenger");
    assert!(state.0.borrow().scenario_runs.len() > runs_before);
    assert_eq!(new_tuner.journal().training_attempt_count(), 1);
}

fn open_with_factory(
    directory: &TestDirectory,
    state: FakeHandle,
    backend: FakeBackend,
    factory: FakeFactory,
    strategy: SequenceStrategy,
) -> Result<TestTuner, TuneError> {
    open_with_stage(directory, state, stage(), backend, factory, strategy)
}

fn open_with_stage(
    directory: &TestDirectory,
    state: FakeHandle,
    stage: SearchStage,
    backend: FakeBackend,
    factory: FakeFactory,
    strategy: SequenceStrategy,
) -> Result<TestTuner, TuneError> {
    Tuner::open_or_resume(
        directory.path(),
        stage,
        91,
        candidate(0.0),
        backend,
        factory,
        EnvelopeGates::new(2.0),
        QuadraticMetric::new(state),
        strategy,
    )
}

#[derive(Debug, Deserialize)]
struct HeadPointer {
    digest: Digest,
}

fn read_head_entry(directory: &TestDirectory) -> JournalEntry {
    let head_bytes = fs::read(directory.path().join("HEAD.json")).expect("read journal head");
    let head: HeadPointer = serde_json::from_slice(&head_bytes).expect("decode journal head");
    let entry_path = directory
        .path()
        .join("entries")
        .join(format!("{}.json", head.digest));
    let entry_bytes = fs::read(entry_path).expect("read journal entry");
    serde_json::from_slice(&entry_bytes).expect("decode journal entry")
}

fn read_entries(directory: &TestDirectory) -> Vec<JournalEntry> {
    let mut entries = fs::read_dir(directory.path().join("entries"))
        .expect("read journal entries")
        .map(|entry| {
            let path = entry.expect("journal entry path").path();
            let bytes = fs::read(path).expect("read journal entry");
            serde_json::from_slice::<JournalEntry>(&bytes).expect("decode journal entry")
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.sequence);
    entries
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExternalMutations {
    open_session: usize,
    prepare: usize,
    start: usize,
    stop: usize,
    cleanup: usize,
    vehicle_bind: usize,
    vehicle_ensure: usize,
    vehicle_apply: usize,
    transition_authorization: usize,
    scenario_runs: usize,
}

impl ExternalMutations {
    fn capture(handle: &FakeHandle) -> Self {
        let state = handle.0.borrow();
        Self {
            open_session: state.open_session_count,
            prepare: state.prepare_count,
            start: state.start_count,
            stop: state.stop_count,
            cleanup: state.cleanup_count,
            vehicle_bind: state.vehicle.bind_count,
            vehicle_ensure: state.vehicle.ensure_count,
            vehicle_apply: state.vehicle.apply_count,
            transition_authorization: state.transition.authorization_count,
            scenario_runs: state.scenario_runs.len(),
        }
    }
}
