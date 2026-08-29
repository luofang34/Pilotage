//! Every receipt in one run has to name that exact run.
//!
//! Preparation, scenario start, candidate activation, and controller
//! readback each return a receipt. A receipt that names another run,
//! another session, another mission, another seed, or another candidate
//! stops the sequence here, before the next external action.
//!
//! SIM / NOT FOR FLIGHT.

#![allow(clippy::expect_used, clippy::panic)]

#[path = "production_binding/rig.rs"]
#[allow(dead_code)]
mod rig;

use flight_tune::{
    CandidateReceipt, Digest, MissionCapability, RunPreparationReceipt, ScenarioStartReceipt,
};
use flight_tune_aviate::runtime::direct::NoDirectControl;
use flight_tune_aviate::runtime::phase::transition::StartStateTolerance;
use flight_tune_aviate::{AviateActionDriver, AviateScenarioDriver};

use rig::{candidate, candidate_digest, mission_document, run_context, runtime_identity};

fn tolerance() -> StartStateTolerance {
    StartStateTolerance {
        position_m: 0.5,
        heading_rad: 0.1,
        speed_mps: 0.2,
        dwell_ns: 500_000_000,
    }
}

/// One admitted run, ready for its receipts to be checked.
fn admitted_run(name: &str, seed: u64) -> (AviateScenarioDriver<NoDirectControl>, Digest) {
    let candidate = candidate(0.06, 0.35, 4.0);
    let digest = candidate_digest(&candidate);
    let document = mission_document(name);
    let context = run_context(0x11, &document, digest, seed);
    let mut driver = AviateScenarioDriver::new(
        runtime_identity(name),
        vec![MissionCapability::KinematicTruth],
        tolerance(),
        NoDirectControl,
    )
    .expect("a driver");
    driver
        .prepare_blocking(&document, &context)
        .expect("admit the run");
    (driver, digest)
}

#[test]
fn a_preparation_receipt_for_another_run_stops_the_next_action() {
    let (driver, _digest) = admitted_run("preparation-receipt", 53);
    let run = driver.admitted_run().expect("the admitted run");
    let intent = run.run_intent_digest();
    let session = run.session_digest();
    let other = Digest::from_bytes([0x63; 32]);

    run.require_preparation(&RunPreparationReceipt {
        session_digest: session,
        run_intent_digest: intent,
    })
    .expect("the exact preparation receipt");
    run.require_preparation(&RunPreparationReceipt {
        session_digest: session,
        run_intent_digest: other,
    })
    .expect_err("another run intent must stop the sequence");
    run.require_preparation(&RunPreparationReceipt {
        session_digest: other,
        run_intent_digest: intent,
    })
    .expect_err("another session must stop the sequence");
}

#[test]
fn a_start_receipt_for_another_mission_or_seed_stops_the_next_action() {
    let (driver, _digest) = admitted_run("start-receipt", 53);
    let run = driver.admitted_run().expect("the admitted run");
    let other = Digest::from_bytes([0x63; 32]);
    let start = ScenarioStartReceipt {
        session_digest: run.session_digest(),
        applied_mission_content_digest: run.mission_content_digest(),
        seed: 53,
        run_intent_digest: run.run_intent_digest(),
    };

    run.require_start(&start).expect("the exact start receipt");
    run.require_start(&ScenarioStartReceipt {
        applied_mission_content_digest: other,
        ..start
    })
    .expect_err("another applied mission must stop the sequence");
    run.require_start(&ScenarioStartReceipt { seed: 54, ..start })
        .expect_err("another run seed must stop the sequence");
    run.require_start(&ScenarioStartReceipt {
        run_intent_digest: other,
        ..start
    })
    .expect_err("another run intent must stop the sequence");
}

#[test]
fn a_candidate_apply_or_readback_mismatch_stops_the_next_action() {
    let (driver, digest) = admitted_run("candidate-receipt", 53);
    let run = driver.admitted_run().expect("the admitted run");
    let other = Digest::from_bytes([0x63; 32]);
    let applied = CandidateReceipt {
        session_digest: run.session_digest(),
        requested_digest: digest,
        applied_digest: digest,
        readback_digest: digest,
        run_intent_digest: Some(run.run_intent_digest()),
    };

    run.require_candidate(&applied)
        .expect("the exact candidate receipt");
    run.require_candidate(&CandidateReceipt {
        applied_digest: other,
        ..applied
    })
    .expect_err("an apply mismatch must stop the sequence");
    run.require_candidate(&CandidateReceipt {
        readback_digest: other,
        ..applied
    })
    .expect_err("a readback mismatch must stop the sequence");
    run.require_candidate(&CandidateReceipt {
        requested_digest: other,
        applied_digest: other,
        readback_digest: other,
        ..applied
    })
    .expect_err("another candidate must stop the sequence");
    run.require_candidate(&CandidateReceipt {
        run_intent_digest: None,
        ..applied
    })
    .expect_err("a receipt with no run intent must stop the sequence");
}
