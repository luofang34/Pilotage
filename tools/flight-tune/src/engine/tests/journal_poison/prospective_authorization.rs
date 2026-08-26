use std::fs;

use pilotage_durable_storage::{FaultController, StorageError};

use super::rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    candidate, stage,
};
use super::{EvidenceSnapshot, TestDirectory, TestTuner, assert_poisoned, assert_snapshot};
use crate::{AttemptRole, JournalEvent, RunExecutionContext, ScenarioSet, TuneError, Tuner};

#[test]
fn a_missing_head_entry_before_cas_keeps_the_old_head_and_poisons() {
    let directory = TestDirectory::new("prospective-missing-head-entry");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone());
    let initial = candidate(0.0);
    let initial_digest = tuner.journal().session().initial_candidate_digest;
    let plan = AttemptRole::TrainingBaseline
        .plan_digest(
            &stage(),
            initial_digest,
            tuner.journal().session().fixed_seed,
        )
        .expect("create run plan");
    let started_digest = entry_digest(&tuner.journal().entries()[0]);
    let started_path = directory
        .path()
        .join("entries")
        .join(format!("{started_digest}.json"));
    let head_before = read_head(&directory);

    let error = tuner
        .journal
        .prepare_attempt_with_before_authorization_for_test(
            AttemptRole::TrainingBaseline,
            &initial,
            plan,
            None,
            || {
                assert_eq!(root_temporary_count(&directory), 1);
                fs::remove_file(&started_path).expect("remove current head entry");
            },
        )
        .expect_err("reject missing current head entry");

    assert_missing_object(error);
    assert_eq!(read_head(&directory), head_before);
    assert_eq!(root_temporary_count(&directory), 0);
    let poisoned = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);
    assert_poisoned(tuner.freeze_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &poisoned);
}

#[test]
fn a_missing_pending_candidate_before_cas_keeps_the_old_head_and_poisons() {
    let directory = TestDirectory::new("prospective-missing-pending-candidate");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone());
    let initial = candidate(0.0);
    let initial_digest = tuner.journal().session().initial_candidate_digest;
    let plan = AttemptRole::TrainingBaseline
        .plan_digest(
            &stage(),
            initial_digest,
            tuner.journal().session().fixed_seed,
        )
        .expect("create run plan");
    let (trial_id, prepared_candidate) = tuner
        .journal
        .prepare_attempt(AttemptRole::TrainingBaseline, &initial, plan, None)
        .expect("prepare pending attempt");
    let candidate_path = directory
        .path()
        .join("candidates")
        .join(format!("{prepared_candidate}.json"));
    let head_before = read_head(&directory);
    let campaign_stage = stage();
    let scenario = &campaign_stage.training_scenarios[0];
    let seed = crate::model::derive_seed(
        tuner.journal().session().fixed_seed,
        ScenarioSet::Training,
        scenario,
        0,
    );
    let context = RunExecutionContext::new(
        tuner.journal().session_digest().expect("session digest"),
        trial_id,
        AttemptRole::TrainingBaseline,
        prepared_candidate,
        None,
        ScenarioSet::Training,
        scenario,
        0,
        seed,
    )
    .expect("run context");
    let run_intent_digest = context.digest().expect("run intent digest");

    let error = tuner
        .journal
        .append_event_with_before_authorization_for_test(
            JournalEvent::RunPrepared {
                trial_id,
                run_index: 0,
                context,
                run_intent_digest,
            },
            || {
                assert_eq!(root_temporary_count(&directory), 1);
                fs::remove_file(&candidate_path).expect("remove pending candidate");
            },
        )
        .expect_err("reject missing pending candidate");

    assert_missing_object(error);
    assert_eq!(read_head(&directory), head_before);
    assert_eq!(root_temporary_count(&directory), 0);
    let poisoned = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);
    assert_poisoned(tuner.qualified_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &poisoned);
}

fn open_tuner(
    directory: &TestDirectory,
    state: FakeHandle,
) -> (TestTuner, super::rig::ObservedViews) {
    let strategy = SequenceStrategy::new(Vec::new());
    let views = strategy.views.clone();
    let tuner = Tuner::open_or_resume_with_faults(
        directory.path(),
        stage(),
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::tracked(2.0, state.clone()),
        QuadraticMetric::new(state),
        strategy,
        FaultController::default(),
    )
    .expect("open tuner");
    (tuner, views)
}

fn entry_digest(entry: &crate::JournalEntry) -> crate::Digest {
    let bytes = serde_json::to_vec(entry).expect("encode journal entry");
    crate::identity::digest_bytes(&bytes)
}

fn read_head(directory: &TestDirectory) -> Vec<u8> {
    fs::read(directory.path().join("HEAD.json")).expect("read journal head")
}

fn root_temporary_count(directory: &TestDirectory) -> usize {
    fs::read_dir(directory.path())
        .expect("read journal root")
        .map(|entry| entry.expect("read root entry"))
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(".pilotage-tmp-"))
        })
        .count()
}

fn assert_missing_object(error: TuneError) {
    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(source.as_ref(), StorageError::Io { source, .. }
                if source.kind() == std::io::ErrorKind::NotFound)
    ));
}
