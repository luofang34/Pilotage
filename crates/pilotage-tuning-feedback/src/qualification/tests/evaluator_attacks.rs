//! Independent qualification of the exact flight-quality evaluator identities.
//!
//! Each case starts from the sealed golden campaign and changes one evaluator
//! identity. A case that reseals the chain proves the verifier's own structural
//! rule; a case that does not reseal proves the identity is bound into the
//! authenticated chain rather than read beside it.

use flight_tune::{ArtifactIdentity, Digest};

use super::fixture::{fixture, try_fixture_with_evaluators};
use super::verify;

/// The verifier keeps its own copy of each evaluator name.
///
/// A mirror that drifted would measure a campaign against a name the harness
/// no longer writes, so the two sides are compared here directly.
#[test]
fn the_verifier_mirrors_each_evaluator_name_byte_for_byte() {
    assert_eq!(
        super::super::campaign::METRIC_IMPLEMENTATION_ID,
        flight_tune::METRIC_IMPLEMENTATION_ID
    );
    assert_eq!(
        super::super::campaign::GATE_IMPLEMENTATION_ID,
        flight_tune::GATE_IMPLEMENTATION_ID
    );
}

#[test]
fn the_sealed_golden_campaign_states_both_evaluator_identities() {
    let evidence = fixture();
    let runtimes = &evidence.journal.head.entry.session.runtimes;

    assert_eq!(runtimes.metric.id, flight_tune::METRIC_IMPLEMENTATION_ID);
    assert_eq!(runtimes.hard_gates.id, flight_tune::GATE_IMPLEMENTATION_ID);
    assert_ne!(runtimes.metric.digest, runtimes.hard_gates.digest);
    verify(&evidence).expect("the unchanged campaign qualifies");
}

#[test]
fn a_changed_metric_identity_fails_independent_qualification() {
    let mut evidence = fixture();
    evidence.journal.head.entry.session.runtimes.metric = identity(
        flight_tune::METRIC_IMPLEMENTATION_ID,
        Digest::from_bytes([73; 32]),
    );

    verify(&evidence).expect_err("a changed metric identity is refused");
}

#[test]
fn a_changed_hard_gate_identity_fails_independent_qualification() {
    let mut evidence = fixture();
    evidence.journal.head.entry.session.runtimes.hard_gates = identity(
        flight_tune::GATE_IMPLEMENTATION_ID,
        Digest::from_bytes([74; 32]),
    );

    verify(&evidence).expect_err("a changed hard gate identity is refused");
}

#[test]
fn exchanged_evaluator_identities_fail_independent_qualification() {
    let sealed = try_fixture_with_evaluators(
        identity(
            flight_tune::GATE_IMPLEMENTATION_ID,
            Digest::from_bytes([33; 32]),
        ),
        identity(
            flight_tune::METRIC_IMPLEMENTATION_ID,
            Digest::from_bytes([34; 32]),
        ),
    );
    let Err(error) = sealed else {
        panic!("exchanged evaluator identities are refused");
    };

    assert!(
        error.to_string().contains("exchanged"),
        "the refusal names another cause: {error}"
    );
}

#[test]
fn one_evaluator_identity_cannot_stand_in_for_the_other() {
    let shared = Digest::from_bytes([33; 32]);
    let sealed = try_fixture_with_evaluators(
        identity(flight_tune::METRIC_IMPLEMENTATION_ID, shared),
        identity(flight_tune::GATE_IMPLEMENTATION_ID, shared),
    );
    let Err(error) = sealed else {
        panic!("a shared evaluator digest is refused");
    };

    assert!(
        error.to_string().contains("stands in for"),
        "the refusal names another cause: {error}"
    );
}

fn identity(id: &str, digest: Digest) -> ArtifactIdentity {
    ArtifactIdentity::new(id, digest).expect("create an evaluator identity")
}
