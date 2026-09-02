use std::fs;

use pilotage_durable_storage::{FaultController, StorageError};

use super::rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    candidate, stage,
};
use super::{EvidenceSnapshot, TestDirectory, TestTuner, assert_poisoned, assert_snapshot};
use crate::{Digest, JournalEvent, TuneError, Tuner};

#[test]
fn an_ancestor_entry_change_poison_stops_the_next_action() {
    let directory = TestDirectory::new("changed-ancestor-entry");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone(), Vec::new());
    tuner
        .run_training_attempts_blocking(0)
        .expect("complete baseline");
    let started = serde_json::to_vec(&tuner.journal().entries()[0]).expect("encode started entry");
    let digest = crate::identity::digest_bytes(&started);
    change_bytes(
        &directory
            .path()
            .join("entries")
            .join(format!("{digest}.json")),
    );
    let expected = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject changed ancestor");

    assert_digest_mismatch(error);
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    assert_poisoned(tuner.freeze_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
}

/// States the interval the catalog audit holds at.
///
/// The audit runs at each action boundary, not at each telemetry sample, so a
/// change made while a sample is in flight does not stop that sample. One
/// sample drives two authorized actions, the observation and the directive it
/// emits, and both take place. The run then refuses to finish and the journal
/// is poisoned, so the changed bytes reach no result and no later action.
///
/// The error variant says which check refused. A sample-time refusal reaches
/// the caller as `TuneError::Adapter`, because the authority hook reports
/// through the scenario runtime; the terminal audit reports the storage
/// failure itself. The contrast is
/// `tests/tuner/scenario_authority.rs::authority_change_during_sampling_precedes_vehicle_action`,
/// where a moved head does stop the sample, because authority is checked at
/// every sample and costs the same at every journal length.
#[test]
fn an_ancestor_entry_change_while_sampling_stops_the_run_at_its_terminal() {
    let directory = TestDirectory::new("sampled-ancestor-entry");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone(), Vec::new());
    let started = serde_json::to_vec(&tuner.journal().entries()[0]).expect("encode started entry");
    let digest = crate::identity::digest_bytes(&started);
    state.0.borrow_mut().change_object_on_sample = Some(
        directory
            .path()
            .join("entries")
            .join(format!("{digest}.json")),
    );

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject changed ancestor");

    let acted = {
        let state = state.0.borrow();
        (
            state.scenario_action_observe_count,
            state.scenario_action_stop_count,
            state.scenario_action_cleanup_count,
        )
    };
    assert_eq!(acted, (2, 1, 1));
    assert_digest_mismatch(error);
    let poisoned = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);
    assert_poisoned(tuner.freeze_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &poisoned);
}

#[test]
fn a_missing_stage_poison_stops_the_next_action() {
    let directory = TestDirectory::new("missing-stage");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone(), Vec::new());
    let stage = tuner.journal().session().stage_digest;
    fs::remove_file(
        directory
            .path()
            .join("stages")
            .join(format!("{stage}.json")),
    )
    .expect("remove stage");
    let expected = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject missing stage");

    assert_missing_object(error);
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    assert_poisoned(tuner.freeze_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
}

#[test]
fn a_changed_noncurrent_candidate_poison_stops_the_next_action() {
    let directory = TestDirectory::new("changed-historical-candidate");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone(), vec![1.0, 0.5]);
    tuner
        .run_training_attempts_blocking(2)
        .expect("complete two challengers");
    let current = tuner.journal().state().training_incumbent;
    let initial = tuner.journal().session().initial_candidate_digest;
    let historical = prepared_candidates(&tuner)
        .find(|digest| *digest != current && *digest != initial)
        .expect("noncurrent prepared candidate");
    change_bytes(
        &directory
            .path()
            .join("candidates")
            .join(format!("{historical}.json")),
    );
    let expected = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);

    let error = tuner
        .freeze_candidate()
        .expect_err("reject changed candidate");

    assert_digest_mismatch(error);
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    assert_poisoned(tuner.qualified_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
}

#[test]
fn a_missing_qualified_candidate_poison_stops_the_read() {
    let directory = TestDirectory::new("missing-qualified-candidate");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone(), vec![0.5]);
    seal_qualified(&mut tuner);
    let selected = tuner
        .journal()
        .state()
        .settlement_candidate(tuner.journal().session().initial_candidate_digest);
    fs::remove_file(
        directory
            .path()
            .join("candidates")
            .join(format!("{selected}.json")),
    )
    .expect("remove qualified candidate");
    let expected = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);

    let error = tuner
        .qualified_candidate()
        .expect_err("reject missing qualified candidate");

    assert_missing_object(error);
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    assert_poisoned(tuner.run_final_qualification_once_blocking());
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
}

fn open_tuner(
    directory: &TestDirectory,
    state: FakeHandle,
    proposals: Vec<f64>,
) -> (TestTuner, super::rig::ObservedViews) {
    let strategy = SequenceStrategy::new(proposals);
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

fn seal_qualified(tuner: &mut TestTuner) {
    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    tuner.run_promotion_once_blocking().expect("run promotion");
    tuner
        .run_final_qualification_once_blocking()
        .expect("run qualification");
}

fn prepared_candidates(tuner: &TestTuner) -> impl Iterator<Item = Digest> + '_ {
    tuner
        .journal()
        .entries()
        .iter()
        .filter_map(|entry| match entry.event {
            JournalEvent::AttemptPrepared { candidate, .. } => Some(candidate),
            _ => None,
        })
}

fn change_bytes(path: &std::path::Path) {
    let mut bytes = fs::read(path).expect("read evidence object");
    bytes.push(b' ');
    fs::write(path, bytes).expect("change evidence object");
}

fn assert_digest_mismatch(error: TuneError) {
    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(source.as_ref(), StorageError::ContentMismatch { context }
                if context.object.is_some())
    ));
}

fn assert_missing_object(error: TuneError) {
    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(source.as_ref(), StorageError::Io { source, .. }
                if source.kind() == std::io::ErrorKind::NotFound)
    ));
}
