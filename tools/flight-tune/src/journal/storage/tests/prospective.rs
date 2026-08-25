use std::collections::BTreeMap;
use std::fs;

use pilotage_durable_storage::{ContentDigest, StorageError};

use super::super::{
    append_entry_with_hook, candidate_digests_to_verify, document_digest, store_candidate,
    store_stage,
};
use super::TestDirectory;
use crate::identity::harness_build_identity;
use crate::journal::{JOURNAL_SCHEMA_VERSION, JournalEntry, JournalEvent, SessionIdentity};
use crate::{
    ArtifactIdentity, AttemptRole, Candidate, CandidateLineage, Digest, ParameterBounds,
    PromotionPolicy, QualificationPolicy, RuntimeIdentities, ScenarioRef, SearchStage, TuneError,
};

#[test]
fn a_changed_prospective_candidate_cannot_publish_a_head() {
    let directory = TestDirectory::new("prospective-candidate-change");
    let (storage, writer) = super::super::open(directory.path()).expect("open journal storage");
    let stage = test_stage();
    let initial = test_candidate();
    let stage_digest = store_stage(&storage, &writer, &stage).expect("store stage");
    let candidate_digest =
        store_candidate(&storage, &writer, &initial).expect("store initial candidate");
    let entry = started_entry(stage_digest, candidate_digest, &initial);
    let entry_digest = document_digest("journal entry", &entry).expect("digest entry");
    let candidate_path = directory
        .path()
        .join("candidates")
        .join(format!("{candidate_digest}.json"));

    let error = append_entry_with_hook(
        &storage,
        &writer,
        &stage,
        std::slice::from_ref(&entry),
        std::slice::from_ref(&entry_digest),
        || change_bytes(&candidate_path),
    )
    .expect_err("reject changed prospective candidate");

    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(
                source.as_ref(),
                StorageError::ContentMismatch { context }
                    if context.object == Some(ContentDigest(*candidate_digest.as_bytes()))
            )
    ));
    assert!(!directory.path().join("HEAD.json").exists());
    assert!(
        directory
            .path()
            .join("entries")
            .join(format!("{entry_digest}.json"))
            .is_file()
    );
}

#[test]
fn candidate_audit_reads_each_digest_once_in_entry_order() {
    let initial = Digest::from_bytes([21; 32]);
    let first = Digest::from_bytes([22; 32]);
    let second = Digest::from_bytes([23; 32]);
    let session = started_entry(Digest::from_bytes([20; 32]), initial, &test_candidate()).session;
    let candidates = [initial, first, first, second, initial];
    let entries = candidates
        .into_iter()
        .enumerate()
        .map(|(sequence, candidate)| JournalEntry {
            schema_version: JOURNAL_SCHEMA_VERSION,
            sequence: u64::try_from(sequence).expect("small sequence"),
            previous: None,
            session: session.clone(),
            event: JournalEvent::AttemptPrepared {
                trial_id: u64::try_from(sequence).expect("small trial identity"),
                role: AttemptRole::TrainingBaseline,
                candidate,
                plan_digest: Digest::from_bytes([24; 32]),
            },
        })
        .collect::<Vec<_>>();

    assert_eq!(
        candidate_digests_to_verify(initial, &entries),
        vec![first, second]
    );
}

fn started_entry(
    stage_digest: Digest,
    candidate_digest: Digest,
    initial: &Candidate,
) -> JournalEntry {
    JournalEntry {
        schema_version: JOURNAL_SCHEMA_VERSION,
        sequence: 0,
        previous: None,
        session: SessionIdentity {
            stage_digest,
            initial_candidate_digest: candidate_digest,
            candidate_lineage: initial.lineage().clone(),
            fixed_seed: 91,
            runtimes: test_runtimes(),
        },
        event: JournalEvent::Started {
            candidate: candidate_digest,
        },
    }
}

fn test_candidate() -> Candidate {
    Candidate::new(
        CandidateLineage {
            schema: "test-candidate-v1".to_owned(),
            base_preset_digest: Digest::from_bytes([7; 32]),
            plant_digest: Digest::from_bytes([8; 32]),
        },
        BTreeMap::from([("gain".to_owned(), 0.0), ("mode".to_owned(), 1.0)]),
    )
    .expect("valid candidate")
}

fn test_stage() -> SearchStage {
    SearchStage {
        id: "test-stage".to_owned(),
        allowlist: BTreeMap::from([(
            "gain".to_owned(),
            ParameterBounds {
                minimum: 0.0,
                maximum: 2.0,
            },
        )]),
        fixed_parameters: BTreeMap::from([("mode".to_owned(), 1.0)]),
        required_hard_gates: vec!["envelope".to_owned()],
        training_scenarios: vec![scenario("training", 1)],
        promotion_scenarios: vec![scenario("promotion", 2)],
        final_qualification_scenarios: vec![scenario("qualification", 3)],
        repetitions: 2,
        promotion: PromotionPolicy {
            minimum_loss_improvement: 0.0,
            minimum_relative_loss_improvement: 0.2,
            maximum_control_effort_increase: 1.0,
        },
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 0.5,
            maximum_p95_loss: 0.5,
            maximum_mean_control_effort: 1.0,
            objective_maxima: BTreeMap::from([("test.response".to_owned(), 0.75)]),
        },
    }
}

fn scenario(id: &str, digest_byte: u8) -> ScenarioRef {
    ScenarioRef {
        id: id.to_owned(),
        digest: Digest::from_bytes([digest_byte; 32]),
        max_samples: 8,
        sample_timeout_ms: 100,
    }
}

fn test_runtimes() -> RuntimeIdentities {
    RuntimeIdentities {
        harness_build: harness_build_identity(),
        strategy: identity("strategy", 11),
        metric: identity("metric", 12),
        hard_gates: identity("hard-gates", 13),
        simulator: identity("simulator", 14),
        airframe: identity("airframe", 15),
        vehicle: identity("vehicle", 16),
    }
}

fn identity(id: &str, digest_byte: u8) -> ArtifactIdentity {
    ArtifactIdentity::new(id, Digest::from_bytes([digest_byte; 32])).expect("valid identity")
}

fn change_bytes(path: &std::path::Path) {
    let mut bytes = fs::read(path).expect("read candidate");
    bytes.push(b' ');
    fs::write(path, bytes).expect("change candidate");
}
