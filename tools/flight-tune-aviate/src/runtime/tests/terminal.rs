//! What one sealed Aviate run states, and what it refuses to state.

use flight_tune::{Digest, MissionTerminal, ScenarioStopContext, ScenarioStopReason};

use crate::runtime::direct::{DIRECT_RUN_EVIDENCE_SCHEMA_VERSION, DirectRunEvidence};
use crate::runtime::terminal::{RUN_SEAL_SCHEMA_VERSION, RunClosure, RunEnding, seal};

use super::identity;

fn stop(reason: ScenarioStopReason) -> ScenarioStopContext {
    ScenarioStopContext {
        reason,
        last_source_sequence: Some(41),
    }
}

fn closure(direct_evidence: Option<DirectRunEvidence>) -> RunClosure {
    RunClosure {
        run_intent_digest: Digest::from_bytes([9; 32]),
        runtime_identity: identity("pilotage-aviate-test-runtime", 6),
        accepted_frames: 42,
        direct_evidence,
        executed_uncertainty: None,
    }
}

fn evidence(run_intent: [u8; 32], runtime_fill: u8) -> DirectRunEvidence {
    DirectRunEvidence {
        schema_version: DIRECT_RUN_EVIDENCE_SCHEMA_VERSION,
        run_intent_digest: Digest::from_bytes(run_intent),
        transport_identity_digest: Digest::from_bytes([8; 32]),
        runtime_identity: identity("pilotage-aviate-test-runtime", runtime_fill),
        records: Vec::new(),
    }
}

#[test]
fn a_sealed_run_binds_its_run_intent_and_runtime() {
    let context = stop(ScenarioStopReason::Mission(MissionTerminal::Complete {
        completed_phases: 3,
    }));
    let sealed = seal(&closure(None), &context).expect("seal the run");
    assert_eq!(sealed.schema_version, RUN_SEAL_SCHEMA_VERSION);
    assert_eq!(sealed.ending, RunEnding::Mission);
    assert_eq!(sealed.accepted_frames, 42);
    assert_eq!(sealed.last_source_sequence, Some(41));
    assert_eq!(sealed.direct_evidence_digest, None);
    assert_eq!(sealed.executed_uncertainty_digest, None);
    sealed
        .require_bound(
            Digest::from_bytes([9; 32]),
            &identity("pilotage-aviate-test-runtime", 6),
        )
        .expect("the seal binds its run and runtime");
    sealed
        .require_bound(
            Digest::from_bytes([1; 32]),
            &identity("pilotage-aviate-test-runtime", 6),
        )
        .expect_err("a seal for another run intent must fail closed");
    sealed
        .require_bound(
            Digest::from_bytes([9; 32]),
            &identity("pilotage-aviate-test-runtime", 5),
        )
        .expect_err("a seal for another runtime must fail closed");
}

#[test]
fn each_stop_reason_seals_its_own_ending() {
    for (reason, ending) in [
        (
            ScenarioStopReason::Mission(MissionTerminal::Complete {
                completed_phases: 3,
            }),
            RunEnding::Mission,
        ),
        (ScenarioStopReason::HardGate, RunEnding::HardGate),
        (ScenarioStopReason::SampleTimeout, RunEnding::SampleTimeout),
        (
            ScenarioStopReason::ExecutionError,
            RunEnding::ExecutionError,
        ),
    ] {
        let sealed = seal(&closure(None), &stop(reason)).expect("seal the run");
        assert_eq!(sealed.ending, ending);
    }
}

#[test]
fn direct_evidence_from_another_run_cannot_be_sealed() {
    let context = stop(ScenarioStopReason::Mission(MissionTerminal::Complete {
        completed_phases: 3,
    }));
    seal(&closure(Some(evidence([9; 32], 6))), &context)
        .expect("evidence that binds this run seals");

    seal(&closure(Some(evidence([1; 32], 6))), &context)
        .expect_err("evidence from another run intent must not seal");
    seal(&closure(Some(evidence([9; 32], 5))), &context)
        .expect_err("evidence from another runtime must not seal");
}

#[test]
fn a_sealed_run_with_direct_evidence_carries_its_identity() {
    let context = stop(ScenarioStopReason::Mission(MissionTerminal::Complete {
        completed_phases: 3,
    }));
    let sealed = seal(&closure(Some(evidence([9; 32], 6))), &context).expect("seal the run");
    let expected = evidence([9; 32], 6).digest().expect("the evidence digest");
    assert_eq!(sealed.direct_evidence_digest, Some(expected));

    let without = seal(&closure(None), &context).expect("seal a run with no direct path");
    assert_ne!(
        sealed.digest().expect("a sealed digest"),
        without.digest().expect("a sealed digest"),
        "a run's direct evidence has to reach its seal"
    );
}
