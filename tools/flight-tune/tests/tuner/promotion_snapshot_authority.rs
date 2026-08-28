use flight_tune::{
    AuthenticatedJournalRecord, Digest, JournalEvent, JournalEvidenceSnapshot, PromotionDecision,
};
use serde::Serialize;
use sha2::{Digest as ShaDigest, Sha256};

use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy, TestDirectory};

const DECISION_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-decision.v1\0";
const SELECTION_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-selection.v1\0";

#[derive(Serialize)]
struct SelectionDocument<'a> {
    decision: &'a PromotionDecision,
    decision_digest: Digest,
    selected_candidate: Option<Digest>,
}

#[test]
fn snapshot_recomputes_the_complete_promotion_closure() {
    let mut changed = promotion_snapshot("snapshot-closure-recompute");
    changed.promotion_closure.decision = PromotionDecision::RejectedNoImprovement {};
    changed.promotion_closure.selected_candidate =
        Some(changed.head.entry.session.initial_candidate_digest);
    refresh_closure(&mut changed);

    assert!(changed.validate().is_err());
}

#[test]
fn stable_snapshot_head_requires_a_chain_ancestor() {
    let exact = promotion_snapshot("snapshot-head-chain");

    let mut zero_sequence = exact.clone();
    zero_sequence.head.entry.sequence = 0;
    refresh_head(&mut zero_sequence);
    assert!(zero_sequence.validate().is_err());

    let mut missing_previous = exact;
    missing_previous.head.entry.previous = None;
    refresh_head(&mut missing_previous);
    assert!(missing_previous.validate().is_err());
}

#[test]
fn snapshot_uses_the_frozen_journal_authority() {
    let mut changed = promotion_snapshot("snapshot-frozen-authority");
    let candidate = Digest::from_bytes([201; 32]);
    changed.authority.frozen_candidate = candidate;
    let JournalEvent::Frozen {
        candidate: event_candidate,
        ..
    } = &mut changed.authority.frozen.entry.event
    else {
        panic!("frozen authority event");
    };
    *event_candidate = candidate;
    refresh_record(&mut changed.authority.frozen);

    assert!(changed.validate().is_err());
}

#[test]
fn snapshot_uses_the_attempt_journal_authority() {
    let mut changed = promotion_snapshot("snapshot-attempt-authority");
    let record = changed
        .authority
        .promotion_frozen
        .as_mut()
        .expect("frozen attempt authority");
    let JournalEvent::AttemptPrepared { trial_id, .. } = &mut record.entry.event else {
        panic!("frozen attempt event");
    };
    *trial_id = trial_id.wrapping_add(1);
    refresh_record(record);

    assert!(changed.validate().is_err());
}

fn promotion_snapshot(label: &str) -> JournalEvidenceSnapshot {
    let directory = TestDirectory::new(label);
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state,
        SequenceStrategy::new(vec![0.5]),
        2.0,
    )
    .expect("open promotion campaign");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    tuner.run_promotion_once_blocking().expect("run promotion");
    tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("verify promotion snapshot")
}

fn refresh_closure(snapshot: &mut JournalEvidenceSnapshot) {
    let closure = &mut snapshot.promotion_closure;
    closure.decision_digest = domain_digest(DECISION_DOMAIN, &closure.decision);
    closure.selection_digest = domain_digest(
        SELECTION_DOMAIN,
        &SelectionDocument {
            decision: &closure.decision,
            decision_digest: closure.decision_digest,
            selected_candidate: closure.selected_candidate,
        },
    );
    closure.closure_digest = closure
        .recompute_closure_digest()
        .expect("recompute changed promotion closure");
    let JournalEvent::PromotionClosed { closure: event } = &mut snapshot.head.entry.event else {
        panic!("promotion snapshot head");
    };
    *event = closure.clone();
    refresh_head(snapshot);
}

fn refresh_head(snapshot: &mut JournalEvidenceSnapshot) {
    let bytes = serde_json::to_vec(&snapshot.head.entry).expect("encode changed journal head");
    snapshot.head.entry_digest = Digest::from_bytes(Sha256::digest(bytes).into());
}

fn refresh_record(record: &mut AuthenticatedJournalRecord) {
    let bytes = serde_json::to_vec(&record.entry).expect("encode changed authority record");
    record.entry_digest = Digest::from_bytes(Sha256::digest(bytes).into());
}

fn domain_digest(domain: &[u8], value: &impl Serialize) -> Digest {
    let encoded = serde_json::to_vec(value).expect("encode domain document");
    let mut bytes = Vec::with_capacity(domain.len().saturating_add(encoded.len()));
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&encoded);
    Digest::from_bytes(Sha256::digest(bytes).into())
}

pub(super) fn assert_promotion_uses_paired_seeds(state: &FakeHandle) {
    let promotion = state
        .0
        .borrow()
        .scenario_runs
        .iter()
        .filter(|(id, _, _)| id == "promotion-gust:1")
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(promotion.len(), 4);
    assert_eq!(promotion[0].1, promotion[2].1);
    assert_eq!(promotion[1].1, promotion[3].1);
}
