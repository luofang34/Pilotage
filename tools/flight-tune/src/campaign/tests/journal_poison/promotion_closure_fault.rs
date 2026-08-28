use pilotage_durable_storage::{
    DurabilityStep, FaultAction, FaultController, FaultRule, StorageOperation,
};

use super::rig::FakeHandle;
use super::{TestDirectory, TestTuner, open_with_faults};
use crate::{AttemptRole, JournalEntry, JournalEvent, PromotionClosure};

#[test]
fn promotion_closure_cas_boundaries_reopen_twice_without_repeating_runs() {
    let (occurrence, expected) = closure_occurrence_and_value();
    assert_closure_boundary(
        "promotion-closure-before-cas",
        FaultController::new([FaultRule::on_occurrence(
            StorageOperation::CompareExchange,
            DurabilityStep::AuthorizationRename,
            occurrence,
            FaultAction::FailBefore,
        )]),
        &expected,
    );
    assert_closure_boundary(
        "promotion-closure-after-cas",
        FaultController::new([
            FaultRule::on_occurrence(
                StorageOperation::CompareExchange,
                DurabilityStep::ParentDirectory,
                occurrence,
                FaultAction::LoseAckAfter,
            ),
            FaultRule::once(
                StorageOperation::CompareExchange,
                DurabilityStep::RecoveryBarrier,
                FaultAction::LoseAckAfter,
            ),
        ]),
        &expected,
    );
}

fn closure_occurrence_and_value() -> (u64, PromotionClosure) {
    let directory = TestDirectory::new("promotion-closure-occurrence");
    let state = FakeHandle::new();
    let (mut tuner, _views) = open_with_faults(&directory, state, FaultController::default());
    drive_to_frozen(&mut tuner);
    tuner.run_promotion_once_blocking().expect("run promotion");
    let entry = tuner
        .journal()
        .entries()
        .last()
        .expect("promotion closure entry");
    let JournalEvent::PromotionClosed { closure } = &entry.event else {
        panic!("campaign did not end at promotion closure");
    };
    (entry.sequence.wrapping_add(1), closure.clone())
}

fn assert_closure_boundary(label: &str, faults: FaultController, expected: &PromotionClosure) {
    let directory = TestDirectory::new(label);
    let state = FakeHandle::new();
    let (mut tuner, _views) = open_with_faults(&directory, state.clone(), faults.clone());
    drive_to_frozen(&mut tuner);
    assert!(tuner.run_promotion_once_blocking().is_err());
    assert!(faults.is_exhausted().expect("read closure fault state"));
    let prepared_count = promotion_preparation_count(tuner.journal().entries());
    drop(tuner);

    for _ in 0..2 {
        let (mut reopened, _views) =
            open_with_faults(&directory, state.clone(), FaultController::default());
        reopened
            .run_promotion_once_blocking()
            .expect("recover promotion closure");
        let snapshot = reopened
            .journal()
            .verified_evidence_snapshot()
            .expect("verify recovered closure");
        assert_eq!(&snapshot.promotion_closure, expected);
        assert_eq!(
            promotion_preparation_count(reopened.journal().entries()),
            prepared_count
        );
        assert_eq!(promotion_closure_count(reopened.journal().entries()), 1);
        drop(reopened);
    }
}

fn drive_to_frozen(tuner: &mut TestTuner) {
    tuner
        .run_training_attempts_blocking(0)
        .expect("complete training baseline");
    tuner.freeze_candidate().expect("freeze candidate");
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

fn promotion_closure_count(entries: &[JournalEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| matches!(entry.event, JournalEvent::PromotionClosed { .. }))
        .count()
}
