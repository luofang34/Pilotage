use std::fs;

use flight_tune::{
    AttemptRole, CandidateTransitionReceipt, CandidateTransitionReference, Digest, JournalEntry,
    JournalEvent, RunExecutionContext, TuneError,
};
use sha2::{Digest as ShaDigest, Sha256};

use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy, TestDirectory};

#[test]
fn replay_rejects_a_coherent_forged_transition_receipt_chain() {
    let (directory, state, strategy, mut entries) = campaign("forged-transition-receipt");
    let index = entries
        .iter()
        .position(|entry| {
            matches!(
                entry.event,
                JournalEvent::CandidateTransitionAuthorized { .. }
            )
        })
        .expect("transition authorization");
    let replacement = match &entries[index].event {
        JournalEvent::CandidateTransitionAuthorized {
            attempt_index,
            reason,
            candidate,
            group,
            receipt,
        } => JournalEvent::CandidateTransitionAuthorized {
            group: group.clone(),
            attempt_index: *attempt_index,
            reason: reason.clone(),
            candidate: *candidate,
            receipt: forged_receipt(receipt),
        },
        _ => panic!("selected event is not a transition authorization"),
    };
    entries[index].event = replacement;
    rewrite_chain(directory.path(), &mut entries, index);

    assert_replay_rejects_without_external_action(&directory, &state, strategy);
}

#[test]
fn replay_rejects_a_coherent_forged_run_context_chain() {
    let (directory, state, strategy, mut entries) = campaign("forged-run-context");
    let forged_reference = entries
        .iter()
        .find_map(|entry| match &entry.event {
            JournalEvent::CandidateTransitionAuthorized { receipt, .. } => {
                Some(forged_receipt(receipt).reference())
            }
            _ => None,
        })
        .expect("transition receipt");
    let index = entries
        .iter()
        .position(|entry| {
            matches!(
                &entry.event,
                JournalEvent::RunPrepared { context, .. }
                    if matches!(context.role(), AttemptRole::TrainingChallenger { .. })
            )
        })
        .expect("challenger run preparation");
    entries[index].event = forged_run_event(&entries[index].event, forged_reference);
    rewrite_chain(directory.path(), &mut entries, index);

    assert_replay_rejects_without_external_action(&directory, &state, strategy);
}

fn campaign(
    label: &str,
) -> (
    TestDirectory,
    FakeHandle,
    SequenceStrategy,
    Vec<JournalEntry>,
) {
    let directory = TestDirectory::new(label);
    let state = FakeHandle::new();
    let strategy = SequenceStrategy::new(vec![0.5]);
    let mut tuner =
        open(directory.path(), state.clone(), strategy.clone(), 2.0).expect("open tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("create challenger evidence");
    let entries = tuner.journal().entries().to_vec();
    drop(tuner);
    (directory, state, strategy, entries)
}

fn forged_receipt(receipt: &CandidateTransitionReceipt) -> CandidateTransitionReceipt {
    let mut document = serde_json::to_value(receipt).expect("encode transition receipt");
    document["adjacency_policy_digest"] = digest_value(91);
    document["planning_context_digest"] = digest_value(92);
    let forged: CandidateTransitionReceipt =
        serde_json::from_value(document.clone()).expect("decode forged transition receipt");
    document["receipt_digest"] = serde_json::to_value(
        forged
            .recompute_digest()
            .expect("recompute forged transition receipt"),
    )
    .expect("encode forged receipt digest");
    let receipt: CandidateTransitionReceipt =
        serde_json::from_value(document).expect("decode coherent forged transition receipt");
    assert_eq!(
        receipt.recompute_digest().expect("verify forged receipt"),
        receipt.receipt_digest()
    );
    receipt
}

fn forged_run_event(event: &JournalEvent, reference: CandidateTransitionReference) -> JournalEvent {
    let JournalEvent::RunPrepared {
        trial_id,
        run_index,
        context,
        ..
    } = event
    else {
        panic!("selected event is not a run preparation");
    };
    let context = forged_context(context, reference);
    let run_intent_digest = context.digest().expect("recompute forged run intent");
    JournalEvent::RunPrepared {
        trial_id: *trial_id,
        run_index: *run_index,
        context,
        run_intent_digest,
    }
}

fn forged_context(
    context: &RunExecutionContext,
    reference: CandidateTransitionReference,
) -> RunExecutionContext {
    let mut document = serde_json::to_value(context).expect("encode run context");
    document["transition_authorization"] =
        serde_json::to_value(reference).expect("encode forged transition reference");
    serde_json::from_value(document).expect("decode coherent forged run context")
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

#[cfg(unix)]
fn set_private_file(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .expect("set private journal object mode");
}

#[cfg(not(unix))]
fn set_private_file(_path: &std::path::Path) {}

fn assert_replay_rejects_without_external_action(
    directory: &TestDirectory,
    state: &FakeHandle,
    strategy: SequenceStrategy,
) {
    let before = ActionSnapshot::capture(state);
    let result = open(directory.path(), state.clone(), strategy, 2.0);
    assert!(matches!(result, Err(TuneError::InvalidJournal { .. })));
    assert_eq!(ActionSnapshot::capture(state), before);
}

fn digest_value(byte: u8) -> serde_json::Value {
    serde_json::to_value(Digest::from_bytes([byte; 32])).expect("encode digest")
}

fn digest_bytes(bytes: &[u8]) -> Digest {
    Digest::from_bytes(Sha256::digest(bytes).into())
}

#[derive(Debug, PartialEq)]
struct ActionSnapshot {
    gain: f64,
    active_candidate: Option<Digest>,
    open_session: usize,
    bind: usize,
    authorize: usize,
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
    lifecycle: Vec<String>,
}

impl ActionSnapshot {
    fn capture(handle: &FakeHandle) -> Self {
        let state = handle.0.borrow();
        Self {
            gain: state.vehicle.gain,
            active_candidate: state.vehicle.active_candidate_digest,
            open_session: state.open_session_count,
            bind: state.vehicle.bind_count,
            authorize: state.transition.authorization_count,
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
            lifecycle: state.lifecycle.clone(),
        }
    }
}
