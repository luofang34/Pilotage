#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use super::*;
use crate::{ArtifactIdentity, CandidateLineage, CandidateTransitionReceipt};

#[test]
fn transition_identity_changes_the_run_intent() {
    let first = context(transition(2));
    let second = context(transition(3));

    assert_ne!(
        first.digest().expect("first digest"),
        second.digest().expect("second digest")
    );
}

#[test]
fn a_non_challenger_cannot_carry_a_transition() {
    let mut value = context(transition(2));
    value.role = AttemptRole::TrainingBaseline { suite_index: 0 };

    assert!(value.validate().is_err());
}

#[test]
fn strict_schema_rejects_an_unknown_field() {
    let value = context(transition(2));
    let mut document = serde_json::to_value(value).expect("encode context");
    document
        .as_object_mut()
        .expect("context object")
        .insert("extra".to_owned(), serde_json::json!(true));

    assert!(serde_json::from_value::<RunExecutionContext>(document).is_err());
}

fn context(reference: CandidateTransitionReference) -> RunExecutionContext {
    RunExecutionContext::new(
        digest(1),
        4,
        AttemptRole::TrainingChallenger { attempt_index: 0, suite_index: 0 },
        reference.target_candidate_digest(),
        Some(reference),
        ScenarioSet::Training,
        &MissionReference {
            revision_id: "calm".to_owned(),
            schema_version: flight_tune::MISSION_SCHEMA_VERSION,
            content_digest: digest(9),
            max_samples: 10,
            sample_timeout_ns: 20_000_000,
        },
        0,
        22,
        0,
    )
    .expect("run context")
}

fn transition(target: u8) -> CandidateTransitionReference {
    let source = candidate(1);
    let target = candidate(target);
    let request = crate::CandidateTransitionRequest::new(
        digest(4),
        &source,
        candidate_digest(&source),
        &target,
        candidate_digest(&target),
        ArtifactIdentity::from_text("validator", "validator-v1").expect("validator"),
        digest(7),
        digest(8),
    )
    .expect("transition request");
    CandidateTransitionReceipt::authorized(&request)
        .expect("transition receipt")
        .reference()
}

fn candidate_digest(candidate: &crate::Candidate) -> Digest {
    let bytes = serde_json::to_vec(candidate).expect("encode candidate");
    crate::identity::digest_bytes(&bytes)
}

fn candidate(value: u8) -> crate::Candidate {
    crate::Candidate::new(
        CandidateLineage {
            schema: "candidate-v1".to_owned(),
            base_preset_digest: digest(5),
            plant_digest: digest(6),
        },
        BTreeMap::from([("gain".to_owned(), f64::from(value))]),
    )
    .expect("candidate")
}

const fn digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}
