use std::fs;

use flight_tune::{
    AttemptRole, Digest, JournalEntry, JournalEvent, PromotionClosure, PromotionDecision, TuneError,
};
use serde::Serialize;
use sha2::{Digest as ShaDigest, Sha256};

use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy, TestDirectory};

const COMPARISON_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-comparison.v1\0";
const DECISION_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-decision.v1\0";
const SELECTION_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-selection.v1\0";

#[test]
fn replay_rejects_every_rechained_promotion_closure_authority() {
    for tamper in [
        ClosureTamper::Policy,
        ClosureTamper::Evaluation,
        ClosureTamper::Proof,
        ClosureTamper::Comparison,
        ClosureTamper::Decision,
        ClosureTamper::Selection,
        ClosureTamper::Digest,
    ] {
        let label = format!("promotion-closure-rechain-{tamper:?}");
        let (directory, state, strategy, mut entries) = promoted_campaign(&label, false);
        let index = entries
            .iter()
            .position(|entry| matches!(entry.event, JournalEvent::PromotionClosed { .. }))
            .expect("promotion closure");
        let initial = entries[index].session.initial_candidate_digest;
        let JournalEvent::PromotionClosed { closure } = &mut entries[index].event else {
            panic!("selected event is not a promotion closure");
        };
        tamper.apply(closure, initial);
        rewrite_chain(directory.path(), &mut entries, index);
        assert_replay_rejects_without_external_action(&directory, &state, strategy);
    }
}

#[test]
fn replay_rejects_each_changed_sealed_anchor() {
    for anchor in [
        SealedAnchor::Promotion,
        SealedAnchor::Evaluation,
        SealedAnchor::Proof,
    ] {
        let label = format!("sealed-anchor-rechain-{anchor:?}");
        let (directory, state, strategy, mut entries) = promoted_campaign(&label, true);
        let index = entries
            .iter()
            .position(|entry| matches!(entry.event, JournalEvent::Sealed { .. }))
            .expect("sealed event");
        let JournalEvent::Sealed {
            promotion_closure_digest,
            final_evaluation_digest,
            final_proof_digest,
            ..
        } = &mut entries[index].event
        else {
            panic!("selected event is not sealed");
        };
        match anchor {
            SealedAnchor::Promotion => *promotion_closure_digest = digest(71),
            SealedAnchor::Evaluation => *final_evaluation_digest = digest(72),
            SealedAnchor::Proof => *final_proof_digest = digest(73),
        }
        rewrite_chain(directory.path(), &mut entries, index);
        assert_replay_rejects_without_external_action(&directory, &state, strategy);
    }
}

#[test]
fn replay_rejects_a_rechained_final_proof_before_its_sealed_anchor() {
    let (directory, state, strategy, mut entries) =
        promoted_campaign("sealed-final-proof-rechain", true);
    let index = entries
        .iter()
        .position(|entry| {
            matches!(
                &entry.event,
                JournalEvent::AttemptCompleted { proof: Some(proof), .. }
                    if proof.role == AttemptRole::FinalQualification
            )
        })
        .expect("final attempt proof");
    let JournalEvent::AttemptCompleted {
        proof: Some(proof), ..
    } = &mut entries[index].event
    else {
        panic!("selected event has no final proof");
    };
    proof.plan_digest = digest(74);
    proof.evaluation_digest = proof
        .recompute_evaluation_digest()
        .expect("recompute final evaluation identity");
    proof.proof_digest = proof
        .recompute_proof_digest()
        .expect("recompute final proof identity");
    rewrite_chain(directory.path(), &mut entries, index);
    assert_replay_rejects_without_external_action(&directory, &state, strategy);
}

#[derive(Clone, Copy, Debug)]
enum ClosureTamper {
    Policy,
    Evaluation,
    Proof,
    Comparison,
    Decision,
    Selection,
    Digest,
}

impl ClosureTamper {
    fn apply(self, closure: &mut PromotionClosure, initial: Digest) {
        match self {
            Self::Policy => closure.policy_digest = digest(61),
            Self::Evaluation => closure.baseline_evaluation_digest = Some(digest(62)),
            Self::Proof => closure.baseline_proof_digest = Some(digest(63)),
            Self::Comparison => {
                let comparison = closure.comparison.as_mut().expect("promotion comparison");
                comparison.loss.mean += 0.01;
                closure.comparison_digest = Some(domain_digest(COMPARISON_DOMAIN, comparison));
            }
            Self::Decision => {
                closure.decision = PromotionDecision::RejectedNoImprovement {};
                closure.selected_candidate = Some(initial);
                rechain_decision_and_selection(closure);
            }
            Self::Selection => {
                closure.selected_candidate = Some(initial);
                closure.selection_digest = selection_digest(closure);
            }
            Self::Digest => {
                closure.closure_digest = digest(64);
                return;
            }
        }
        closure.closure_digest = closure
            .recompute_closure_digest()
            .expect("recompute changed closure");
    }
}

#[derive(Clone, Copy, Debug)]
enum SealedAnchor {
    Promotion,
    Evaluation,
    Proof,
}

fn rechain_decision_and_selection(closure: &mut PromotionClosure) {
    closure.decision_digest = domain_digest(DECISION_DOMAIN, &closure.decision);
    closure.selection_digest = selection_digest(closure);
}

fn selection_digest(closure: &PromotionClosure) -> Digest {
    domain_digest(
        SELECTION_DOMAIN,
        &SelectionDocument {
            decision: &closure.decision,
            decision_digest: closure.decision_digest,
            selected_candidate: closure.selected_candidate,
        },
    )
}

#[derive(Serialize)]
struct SelectionDocument<'a> {
    decision: &'a PromotionDecision,
    decision_digest: Digest,
    selected_candidate: Option<Digest>,
}

fn promoted_campaign(
    label: &str,
    seal: bool,
) -> (
    TestDirectory,
    FakeHandle,
    SequenceStrategy,
    Vec<JournalEntry>,
) {
    let directory = TestDirectory::new(label);
    let state = FakeHandle::new();
    let strategy = SequenceStrategy::new(vec![0.5]);
    let mut tuner = open(directory.path(), state.clone(), strategy.clone(), 2.0)
        .expect("open promotion campaign");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    tuner.run_promotion_once_blocking().expect("run promotion");
    if seal {
        tuner
            .run_final_qualification_once_blocking()
            .expect("run final qualification");
    }
    let entries = tuner.journal().entries().to_vec();
    drop(tuner);
    (directory, state, strategy, entries)
}

fn rewrite_chain(root: &std::path::Path, entries: &mut [JournalEntry], changed: usize) {
    let mut previous = None;
    for (index, entry) in entries.iter_mut().enumerate() {
        entry.previous = previous;
        let bytes = serde_json::to_vec(entry).expect("encode rebuilt journal entry");
        let digest = digest_bytes(&bytes);
        if index >= changed {
            let path = root.join("entries").join(format!("{digest}.json"));
            fs::write(&path, bytes).expect("write rebuilt journal entry");
            set_private_file(&path);
        }
        previous = Some(digest);
    }
    let head = previous.expect("rebuilt journal head");
    let bytes = serde_json::to_vec(&serde_json::json!({ "digest": head }))
        .expect("encode rebuilt journal head");
    fs::write(root.join("HEAD.json"), bytes).expect("write rebuilt journal head");
}

fn assert_replay_rejects_without_external_action(
    directory: &TestDirectory,
    state: &FakeHandle,
    strategy: SequenceStrategy,
) {
    let before = format!("{:?}", *state.0.borrow());
    let result = open(directory.path(), state.clone(), strategy, 2.0);
    assert!(matches!(result, Err(TuneError::InvalidJournal { .. })));
    assert_eq!(format!("{:?}", *state.0.borrow()), before);
}

fn domain_digest(domain: &[u8], value: &impl Serialize) -> Digest {
    let encoded = serde_json::to_vec(value).expect("encode domain document");
    let mut bytes = Vec::with_capacity(domain.len().saturating_add(encoded.len()));
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&encoded);
    digest_bytes(&bytes)
}

fn digest_bytes(bytes: &[u8]) -> Digest {
    Digest::from_bytes(Sha256::digest(bytes).into())
}

fn digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}

#[cfg(unix)]
fn set_private_file(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .expect("set private journal object mode");
}

#[cfg(not(unix))]
fn set_private_file(_path: &std::path::Path) {}
