use std::fs;
use std::path::Path;

use flight_tune::{AttemptRole, Digest, JournalEntry, JournalEvent};

use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy, TestDirectory};

#[test]
fn promotion_closure_reopens_twice_without_repeating_hidden_runs() {
    let directory = TestDirectory::new("promotion-closure-reopen");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.5]),
        2.0,
    )
    .expect("open promotion campaign");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    let decision = tuner.run_promotion_once_blocking().expect("run promotion");
    let closure = tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("read promotion closure")
        .promotion_closure;
    let prepared_count = promotion_preparation_count(tuner.journal().entries());
    drop(tuner);

    for _ in 0..2 {
        let mut reopened = open(
            directory.path(),
            state.clone(),
            SequenceStrategy::new(vec![0.5]),
            2.0,
        )
        .expect("reopen promotion campaign");
        assert_eq!(
            reopened
                .run_promotion_once_blocking()
                .expect("read saved promotion"),
            decision
        );
        assert_eq!(
            reopened
                .journal()
                .verified_evidence_snapshot()
                .expect("verify reopened closure")
                .promotion_closure,
            closure
        );
        assert_eq!(
            promotion_preparation_count(reopened.journal().entries()),
            prepared_count
        );
        assert_eq!(
            reopened
                .journal()
                .entries()
                .iter()
                .filter(|entry| matches!(entry.event, JournalEvent::PromotionClosed { .. }))
                .count(),
            1
        );
        drop(reopened);
    }
}

#[test]
fn orphaned_promotion_closure_rebuilds_without_repeating_hidden_runs() {
    let directory = TestDirectory::new("promotion-closure-orphan");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.5]),
        2.0,
    )
    .expect("open promotion campaign");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    let decision = tuner.run_promotion_once_blocking().expect("run promotion");
    let snapshot = tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("read promotion closure");
    let rewind_digest = tuner
        .journal()
        .entries()
        .iter()
        .find_map(|entry| {
            matches!(entry.event, JournalEvent::PromotionClosed { .. })
                .then_some(entry.previous)
                .flatten()
        })
        .expect("promotion closure previous digest");
    let prepared_count = promotion_preparation_count(tuner.journal().entries());
    drop(tuner);
    write_head(directory.path(), rewind_digest);

    let mut reopened = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.5]),
        2.0,
    )
    .expect("reopen before promotion closure");
    assert!(reopened.journal().verified_evidence_snapshot().is_err());
    assert_eq!(
        reopened
            .run_promotion_once_blocking()
            .expect("rebuild promotion closure"),
        decision
    );
    assert_recovered_closure(&reopened, &snapshot, prepared_count);
    drop(reopened);

    let mut reopened = open(
        directory.path(),
        state,
        SequenceStrategy::new(vec![0.5]),
        2.0,
    )
    .expect("reopen rebuilt promotion closure");
    assert_eq!(
        reopened
            .run_promotion_once_blocking()
            .expect("read rebuilt promotion closure"),
        decision
    );
    assert_recovered_closure(&reopened, &snapshot, prepared_count);
}

fn assert_recovered_closure(
    tuner: &super::TestTuner,
    expected: &flight_tune::JournalEvidenceSnapshot,
    prepared_count: usize,
) {
    let actual = tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("verify rebuilt promotion closure");
    assert_eq!(actual.promotion_closure, expected.promotion_closure);
    assert_eq!(
        actual.promotion_baseline.trial_id,
        expected.promotion_baseline.trial_id
    );
    assert_eq!(actual.promotion_frozen, expected.promotion_frozen);
    assert_eq!(
        promotion_preparation_count(tuner.journal().entries()),
        prepared_count
    );
    assert_eq!(
        tuner
            .journal()
            .entries()
            .iter()
            .filter(|entry| matches!(entry.event, JournalEvent::PromotionClosed { .. }))
            .count(),
        1
    );
}

fn write_head(root: &Path, digest: Digest) {
    let bytes =
        serde_json::to_vec(&serde_json::json!({ "digest": digest })).expect("encode journal head");
    fs::write(root.join("HEAD.json"), bytes).expect("rewind journal head");
}

fn promotion_preparation_count(entries: &[JournalEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.event,
                JournalEvent::RunPrepared { ref context, .. }
                    if matches!(
                        context.role(),
                        AttemptRole::PromotionBaseline | AttemptRole::PromotionFrozen
                    )
            )
        })
        .count()
}
