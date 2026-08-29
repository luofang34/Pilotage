#![allow(clippy::expect_used)]

use std::cell::Cell;
use std::collections::BTreeMap;

use super::super::TransitionBindingReceipt;
use super::*;
use crate::identity::digest_bytes;
use crate::{
    AdapterError, CandidateLineage, CandidateReceipt, SimulatorCapability, SimulatorSessionReceipt,
    SimulatorVehicleAdapter, VehicleBindingReceipt,
};

#[test]
fn receipt_binds_every_transition_input() {
    let request = request(candidate(1.0), candidate(2.0));
    let receipt = CandidateTransitionReceipt::authorized(&request).expect("create receipt");
    assert!(receipt.validate_for(&request).is_ok());
    assert_eq!(
        receipt.recompute_digest().expect("digest"),
        receipt.receipt_digest()
    );
    assert!(
        receipt
            .reference()
            .validate_for_runtime(
                digest(1),
                request.source_candidate_digest(),
                request.target_candidate_digest(),
                &validator(),
                digest(3),
                digest(4)
            )
            .is_ok()
    );
    assert!(
        receipt
            .reference()
            .validate_for_runtime(
                digest(1),
                digest(9),
                request.target_candidate_digest(),
                &validator(),
                digest(3),
                digest(4)
            )
            .is_err()
    );

    let other = CandidateTransitionRequest::new(
        digest(1),
        request.source(),
        request.source_candidate_digest(),
        &candidate(3.0),
        candidate_digest(&candidate(3.0)),
        validator(),
        digest(3),
        digest(4),
    )
    .expect("other request");
    assert!(receipt.validate_for(&other).is_err());
}

#[test]
fn unchanged_lineage_and_parameter_name_changes_fail_closed() {
    let source = candidate(1.0);
    assert!(make_request(&source, &source).is_err());

    let changed_lineage = Candidate::new(
        CandidateLineage {
            schema: "other".to_owned(),
            base_preset_digest: digest(7),
            plant_digest: digest(8),
        },
        BTreeMap::from([("gain".to_owned(), 2.0)]),
    )
    .expect("candidate");
    assert!(make_request(&source, &changed_lineage).is_err());

    let changed_name = Candidate::new(
        source.lineage().clone(),
        BTreeMap::from([("rate".to_owned(), 2.0)]),
    )
    .expect("candidate");
    assert!(make_request(&source, &changed_name).is_err());
}

#[test]
fn candidate_digest_mismatch_fails_closed() {
    let source = candidate(1.0);
    let target = candidate(2.0);
    for (source_digest, target_digest) in [
        (digest(9), candidate_digest(&target)),
        (candidate_digest(&source), digest(9)),
    ] {
        assert!(
            CandidateTransitionRequest::new(
                digest(1),
                &source,
                source_digest,
                &target,
                target_digest,
                validator(),
                digest(3),
                digest(4),
            )
            .is_err()
        );
    }
}

#[test]
fn receipt_schema_and_unknown_fields_fail_closed() {
    let request = request(candidate(1.0), candidate(2.0));
    let receipt = CandidateTransitionReceipt::authorized(&request).expect("create receipt");
    let mut value = serde_json::to_value(&receipt).expect("encode receipt");
    value["schema_version"] = serde_json::json!(0);
    let downgraded: CandidateTransitionReceipt =
        serde_json::from_value(value.clone()).expect("decode receipt");
    assert!(downgraded.validate_for(&request).is_err());
    value["unknown"] = serde_json::json!(true);
    assert!(serde_json::from_value::<CandidateTransitionReceipt>(value).is_err());
}

#[test]
fn binding_checks_session_validator_and_policy_before_adapter_call() {
    let calls = Cell::new(0);
    let capability = SimulatorCapability::new(SimulatorSessionReceipt {
        session_digest: digest(1),
        simulator_digest: digest(5),
        airframe_digest: digest(6),
    });
    let binding = capability
        .bind_vehicle_with_transition(
            AcceptingAdapter { calls: &calls },
            VehicleBindingReceipt {
                session_digest: digest(1),
                vehicle_digest: digest(2),
                scenario_runtime_digest: digest(7),
            },
            TransitionBindingReceipt::new(digest(1), validator(), digest(3))
                .expect("transition binding"),
        )
        .expect("vehicle binding");
    let request = request(candidate(1.0), candidate(2.0));
    assert!(binding.authorize_candidate_transition(&request).is_ok());
    assert_eq!(calls.get(), 1);

    let wrong_policy = CandidateTransitionRequest::new(
        digest(1),
        request.source(),
        request.source_candidate_digest(),
        request.target(),
        request.target_candidate_digest(),
        validator(),
        digest(9),
        digest(4),
    )
    .expect("request");
    assert!(
        binding
            .authorize_candidate_transition(&wrong_policy)
            .is_err()
    );
    assert_eq!(calls.get(), 1);
}

#[test]
fn transition_binding_rejects_another_session() {
    let capability = SimulatorCapability::new(SimulatorSessionReceipt {
        session_digest: digest(1),
        simulator_digest: digest(5),
        airframe_digest: digest(6),
    });
    assert!(
        capability
            .bind_vehicle_with_transition(
                AcceptingAdapter {
                    calls: &Cell::new(0)
                },
                VehicleBindingReceipt {
                    session_digest: digest(1),
                    vehicle_digest: digest(2),
                    scenario_runtime_digest: digest(7),
                },
                TransitionBindingReceipt::new(digest(9), validator(), digest(3))
                    .expect("transition binding"),
            )
            .is_err()
    );
}

#[test]
fn planning_context_is_domain_separated_and_complete() {
    let group = group_binding();
    let context =
        planning_context_digest(digest(1), digest(2), &group).expect("planning context");
    assert!(!context.is_zero());
    assert_ne!(context, digest_bytes(&[1; 32]));
    assert!(planning_context_digest(digest(0), digest(2), &group).is_err());
    let mut empty = group_binding();
    empty.suite_digest = crate::Digest::from_bytes([0; 32]);
    assert!(planning_context_digest(digest(1), digest(2), &empty).is_err());
}

struct AcceptingAdapter<'a> {
    calls: &'a Cell<u32>,
}

impl SimulatorVehicleAdapter for AcceptingAdapter<'_> {
    fn authorize_candidate_transition(
        &self,
        request: &CandidateTransitionRequest,
    ) -> Result<CandidateTransitionReceipt, AdapterError> {
        self.calls.set(self.calls.get().wrapping_add(1));
        CandidateTransitionReceipt::authorized(request)
            .map_err(|error| AdapterError::new(error.to_string()))
    }

    fn ensure_settled_candidate_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _candidate: &Candidate,
        _candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        Err(AdapterError::new("test does not apply a candidate"))
    }
}

fn request(source: Candidate, target: Candidate) -> CandidateTransitionRequest {
    make_request(&source, &target).expect("request")
}

fn make_request(
    source: &Candidate,
    target: &Candidate,
) -> Result<CandidateTransitionRequest, TuneError> {
    CandidateTransitionRequest::new(
        digest(1),
        source,
        candidate_digest(source),
        target,
        candidate_digest(target),
        validator(),
        digest(3),
        digest(4),
    )
}

fn candidate(value: f64) -> Candidate {
    Candidate::new(
        CandidateLineage {
            schema: "test".to_owned(),
            base_preset_digest: digest(7),
            plant_digest: digest(8),
        },
        BTreeMap::from([("gain".to_owned(), value)]),
    )
    .expect("candidate")
}

fn candidate_digest(candidate: &Candidate) -> Digest {
    digest_bytes(&serde_json::to_vec(candidate).expect("encode candidate"))
}

fn validator() -> ArtifactIdentity {
    ArtifactIdentity::new("test-transition-validator", digest(10)).expect("validator")
}

const fn digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}

/// One binding a planning context can be calculated over.
fn group_binding() -> crate::SearchGroupBinding {
    crate::SearchGroupBinding {
        group_id: "dynamics".to_owned(),
        suite_id: "direct-response".to_owned(),
        suite_index: 0,
        suite_digest: digest(31),
    }
}
