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
use crate::model::derive_seed;
use crate::score::aggregate_runs;
use crate::{
    ArtifactIdentity, AttemptRole, Candidate, CandidateEvaluation, CandidateLineage,
    CandidateTransitionReceipt, CandidateTransitionRequest, Digest, GateOutcome, HardGateFailure,
    Journal, OperationStatus, ParameterBounds, PromotionPolicy, QualificationPolicy,
    RunExecutionContext, RunRecord, RuntimeIdentities, ScenarioRef, ScenarioSet, SearchStage,
    TuneError,
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
fn resume_reads_an_authorized_target_before_attempt_preparation() {
    let directory = TestDirectory::new("authorized-target-resume");
    let stage = test_stage();
    let initial = test_candidate();
    let target = initial
        .with_parameter("gain", 0.5)
        .expect("transition target");
    let runtimes = test_runtimes();
    let mut journal =
        Journal::open_or_create(directory.path(), &stage, 91, runtimes.clone(), &initial)
            .expect("start journal");
    record_passing_baseline(&mut journal, &stage, &initial);
    let initial_digest = document_digest("candidate", &initial).expect("initial digest");
    let target_digest = document_digest("candidate", &target).expect("target digest");
    let role = AttemptRole::TrainingChallenger { attempt_index: 0 };
    let plan = role
        .plan_digest(&stage, target_digest, 91)
        .expect("challenger plan");
    let planning = crate::adapter::planning_context_digest(journal.session().stage_digest, plan)
        .expect("planning context");
    let request = CandidateTransitionRequest::new(
        journal.session_digest().expect("session digest"),
        &initial,
        initial_digest,
        &target,
        target_digest,
        runtimes.transition_validator.clone(),
        runtimes.adjacency_policy_digest,
        planning,
    )
    .expect("transition request");
    let receipt = CandidateTransitionReceipt::authorized(&request).expect("transition receipt");
    journal
        .authorize_training_transition(0, "increase gain", &target, receipt)
        .expect("save transition authorization");
    drop(journal);
    change_bytes(
        &directory
            .path()
            .join("candidates")
            .join(format!("{target_digest}.json")),
    );

    let error = Journal::open_or_create(directory.path(), &stage, 91, runtimes, &initial)
        .err()
        .expect("reject changed authorized target");

    assert!(matches!(error, TuneError::Storage { .. }));
}

#[test]
fn a_hard_gate_failed_baseline_cannot_authorize_a_challenger() {
    let directory = TestDirectory::new("hard-gate-baseline-authorization");
    let stage = test_stage();
    let initial = test_candidate();
    let target = initial
        .with_parameter("gain", 0.5)
        .expect("transition target");
    let runtimes = test_runtimes();
    let mut journal =
        Journal::open_or_create(directory.path(), &stage, 91, runtimes.clone(), &initial)
            .expect("start journal");
    record_hard_gate_failed_baseline(&mut journal, &stage, &initial);

    reject_challenger_authorization(&mut journal, &stage, &initial, &target);
    drop(journal);

    let resumed = Journal::open_or_create(directory.path(), &stage, 91, runtimes, &initial)
        .expect("resume failed baseline");
    assert_no_transition_authorization(&resumed);
}

#[test]
fn a_quarantined_baseline_cannot_authorize_a_challenger() {
    let directory = TestDirectory::new("quarantined-baseline-authorization");
    let stage = test_stage();
    let initial = test_candidate();
    let target = initial
        .with_parameter("gain", 0.5)
        .expect("transition target");
    let runtimes = test_runtimes();
    let mut journal =
        Journal::open_or_create(directory.path(), &stage, 91, runtimes.clone(), &initial)
            .expect("start journal");
    let (trial_id, _) = prepare_baseline(&mut journal, &stage, &initial);
    journal
        .quarantine_attempt(trial_id, "baseline simulator failure")
        .expect("quarantine baseline");
    record_successful_cleanup(&mut journal, trial_id);

    reject_challenger_authorization(&mut journal, &stage, &initial, &target);
    drop(journal);

    let resumed = Journal::open_or_create(directory.path(), &stage, 91, runtimes, &initial)
        .expect("resume quarantined baseline");
    assert_no_transition_authorization(&resumed);
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
                transition: None,
            },
        })
        .collect::<Vec<_>>();

    assert_eq!(
        candidate_digests_to_verify(initial, &entries),
        vec![first, second]
    );
}

fn record_passing_baseline(journal: &mut Journal, stage: &SearchStage, candidate: &Candidate) {
    let (trial_id, candidate_digest) = prepare_baseline(journal, stage, candidate);
    let scenario = &stage.training_scenarios[0];
    let runs = (0..stage.repetitions)
        .map(|repetition| {
            let seed = prepare_training_run(journal, stage, trial_id, candidate_digest, repetition);
            RunRecord {
                scenario_set: ScenarioSet::Training,
                scenario_id: scenario.id.clone(),
                repetition,
                seed,
                loss: 0.5,
                control_effort: 0.2,
                objectives: BTreeMap::new(),
                passed_hard_gates: stage.required_hard_gates.clone(),
            }
        })
        .collect::<Vec<_>>();
    let aggregate = aggregate_runs(&runs, ScenarioSet::Training).expect("aggregate baseline");
    journal
        .complete_attempt(
            trial_id,
            CandidateEvaluation::Passed { aggregate, runs },
            Some(true),
        )
        .expect("complete passing baseline");
    record_successful_cleanup(journal, trial_id);
}

fn record_hard_gate_failed_baseline(
    journal: &mut Journal,
    stage: &SearchStage,
    candidate: &Candidate,
) {
    let (trial_id, candidate_digest) = prepare_baseline(journal, stage, candidate);
    let scenario = &stage.training_scenarios[0];
    let seed = prepare_training_run(journal, stage, trial_id, candidate_digest, 0);
    let failure = HardGateFailure {
        scenario_set: ScenarioSet::Training,
        scenario_id: scenario.id.clone(),
        repetition: 0,
        seed,
        sample_sequence: 1,
        elapsed_ms: 10,
        gate: GateOutcome::fail("envelope", "the vehicle left the test envelope"),
    };
    journal
        .complete_attempt(
            trial_id,
            CandidateEvaluation::HardGateFailed {
                failure,
                completed_runs: Vec::new(),
            },
            Some(false),
        )
        .expect("complete failed baseline");
    record_successful_cleanup(journal, trial_id);
}

fn prepare_baseline(
    journal: &mut Journal,
    stage: &SearchStage,
    candidate: &Candidate,
) -> (u64, Digest) {
    let candidate_digest = document_digest("candidate", candidate).expect("candidate digest");
    let role = AttemptRole::TrainingBaseline;
    let plan = role
        .plan_digest(stage, candidate_digest, journal.session().fixed_seed)
        .expect("baseline plan");
    journal
        .prepare_attempt(role, candidate, plan, None)
        .expect("prepare baseline")
}

fn prepare_training_run(
    journal: &mut Journal,
    stage: &SearchStage,
    trial_id: u64,
    candidate_digest: Digest,
    repetition: u32,
) -> u64 {
    let scenario = &stage.training_scenarios[0];
    let seed = derive_seed(
        journal.session().fixed_seed,
        ScenarioSet::Training,
        scenario,
        repetition,
    );
    let context = RunExecutionContext::new(
        journal.session_digest().expect("session digest"),
        trial_id,
        AttemptRole::TrainingBaseline,
        candidate_digest,
        None,
        ScenarioSet::Training,
        scenario,
        repetition,
        seed,
    )
    .expect("baseline run context");
    journal
        .prepare_run(u64::from(repetition), &context)
        .expect("prepare baseline run");
    seed
}

fn reject_challenger_authorization(
    journal: &mut Journal,
    stage: &SearchStage,
    source: &Candidate,
    target: &Candidate,
) {
    let receipt = transition_receipt(journal, stage, source, target, 0);
    let entry_count = journal.entries().len();
    let error = journal
        .authorize_training_transition(0, "increase gain", target, receipt)
        .expect_err("reject challenger authorization");

    assert!(matches!(error, TuneError::InvalidJournal { .. }));
    assert_eq!(journal.entries().len(), entry_count);
    assert_no_transition_authorization(journal);
}

fn transition_receipt(
    journal: &Journal,
    stage: &SearchStage,
    source: &Candidate,
    target: &Candidate,
    attempt_index: u64,
) -> CandidateTransitionReceipt {
    let source_digest = document_digest("candidate", source).expect("source digest");
    let target_digest = document_digest("candidate", target).expect("target digest");
    let plan = AttemptRole::TrainingChallenger { attempt_index }
        .plan_digest(stage, target_digest, journal.session().fixed_seed)
        .expect("challenger plan");
    let planning = crate::adapter::planning_context_digest(journal.session().stage_digest, plan)
        .expect("planning context");
    let request = CandidateTransitionRequest::new(
        journal.session_digest().expect("session digest"),
        source,
        source_digest,
        target,
        target_digest,
        journal.session().runtimes.transition_validator.clone(),
        journal.session().runtimes.adjacency_policy_digest,
        planning,
    )
    .expect("transition request");
    CandidateTransitionReceipt::authorized(&request).expect("transition receipt")
}

fn record_successful_cleanup(journal: &mut Journal, trial_id: u64) {
    journal
        .record_cleanup(
            trial_id,
            OperationStatus::Succeeded,
            OperationStatus::Succeeded,
        )
        .expect("record successful cleanup");
}

fn assert_no_transition_authorization(journal: &Journal) {
    assert!(journal.entries().iter().all(|entry| !matches!(
        entry.event,
        JournalEvent::CandidateTransitionAuthorized { .. }
    )));
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
        transition_validator: identity("transition-validator", 17),
        adjacency_policy_digest: Digest::from_bytes([18; 32]),
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
