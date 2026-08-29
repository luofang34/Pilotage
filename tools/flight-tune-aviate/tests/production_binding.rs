//! The production Aviate vehicle binding and its run identity.
//!
//! Every test here drives the public contract: the factory, the adapter,
//! the transition validator, and the sealed runtime identity. Nothing
//! reaches inside them, because what the campaign relies on is exactly
//! what a caller can see.
//!
//! SIM / NOT FOR FLIGHT.

#![allow(clippy::expect_used, clippy::panic)]

#[path = "production_binding/rig.rs"]
mod rig;

use flight_tune::{
    CandidateReceipt, CandidateTransitionRequest, Digest, MissionCapability, RunPreparationReceipt,
    ScenarioStartReceipt, SimulatorVehicleAdapter, SimulatorVehicleFactory,
    scenario_runtime_identity,
};
use flight_tune_aviate::runtime::direct::NoDirectControl;
use flight_tune_aviate::runtime::phase::transition::StartStateTolerance;
use flight_tune_aviate::{
    AviateActionDriver, AviateScenarioDriver, AviateVehicleFactory, aviate_action_port_identity,
    bind_run_intent, require_run_intent,
};

use rig::{
    APPLY_ACCEL, adapter, adapter_with_log, candidate, candidate_digest, capability,
    mission_document, run_context, runtime_identity, validator,
};

const PLANNING_CONTEXT: Digest = Digest::from_bytes([0x41; 32]);

fn transition_request(
    session: u8,
    source: &flight_tune::Candidate,
    target: &flight_tune::Candidate,
) -> CandidateTransitionRequest {
    let validator = validator();
    CandidateTransitionRequest::new(
        Digest::from_bytes([session; 32]),
        source,
        candidate_digest(source),
        target,
        candidate_digest(target),
        validator.identity().clone(),
        validator.policy_digest(),
        PLANNING_CONTEXT,
    )
    .expect("a complete transition request")
}

#[test]
fn a_later_transition_is_checked_against_the_current_incumbent() {
    let first = candidate(0.06, 0.35, 4.0);
    let second = candidate(0.06, 0.35, 4.4);
    let third = candidate(0.06, 0.35, 4.8);
    let mut adapter = adapter(0x11);
    let capability = capability(0x11);

    adapter
        .ensure_settled_candidate_blocking(&capability, &first, candidate_digest(&first))
        .expect("settle the first candidate");
    adapter
        .authorize_candidate_transition(&transition_request(0x11, &first, &second))
        .expect("authorize the first transition from the incumbent");

    // The incumbent is still the first candidate, so a request that names
    // the second as its source is a request against a vehicle state that
    // does not exist.
    let stale = adapter.authorize_candidate_transition(&transition_request(0x11, &second, &third));
    let detail = stale.expect_err("a stale source must fail").to_string();
    assert!(detail.contains("current incumbent"), "{detail}");

    adapter
        .ensure_settled_candidate_blocking(&capability, &second, candidate_digest(&second))
        .expect("settle the second candidate");
    adapter
        .authorize_candidate_transition(&transition_request(0x11, &second, &third))
        .expect("authorize the later transition from the new incumbent");
}

#[test]
fn a_transition_further_than_one_adjacent_step_fails_closed() {
    let first = candidate(0.06, 0.35, 4.0);
    let distant = candidate(0.06, 0.35, 40.0);
    let mut adapter = adapter(0x11);
    adapter
        .ensure_settled_candidate_blocking(&capability(0x11), &first, candidate_digest(&first))
        .expect("settle the incumbent");
    let detail = adapter
        .authorize_candidate_transition(&transition_request(0x11, &first, &distant))
        .expect_err("a distant transition must fail")
        .to_string();
    assert!(detail.contains(APPLY_ACCEL), "{detail}");
}

#[test]
fn an_invalid_complete_feel_profile_is_refused_before_any_controller_write() {
    // A negative apply acceleration is a complete profile the validator
    // refuses, so the mapping fails before the controller is touched.
    let invalid = candidate(0.06, 0.35, -1.0);
    let (mut adapter, controller) = adapter_with_log(0x11);
    let before = controller.applies();

    let detail = adapter
        .ensure_settled_candidate_blocking(&capability(0x11), &invalid, candidate_digest(&invalid))
        .expect_err("an invalid mapped profile must fail")
        .to_string();
    assert!(detail.contains("control-feel"), "{detail}");
    assert_eq!(controller.applies(), before, "no profile may be written");
    assert_eq!(adapter.settled_candidate_digest(), None);

    // The same invalid target is refused as a transition target too, and
    // again with no write.
    let incumbent = candidate(0.06, 0.35, 4.0);
    adapter
        .authorize_candidate_transition(&transition_request(0x11, &incumbent, &invalid))
        .expect_err("an invalid mapped target must fail");
    assert_eq!(controller.applies(), before, "no profile may be written");
}

#[test]
fn another_transition_receipt_fails_closed() {
    let first = candidate(0.06, 0.35, 4.0);
    let second = candidate(0.06, 0.35, 4.4);
    let mut adapter = adapter(0x11);
    adapter
        .ensure_settled_candidate_blocking(&capability(0x11), &first, candidate_digest(&first))
        .expect("settle the incumbent");

    // A request that names another validator identity is a request this
    // vehicle never agreed to enforce.
    let other = CandidateTransitionRequest::new(
        Digest::from_bytes([0x11; 32]),
        &first,
        candidate_digest(&first),
        &second,
        candidate_digest(&second),
        rig::identity("pilotage-aviate-other-validator", 0x61),
        validator().policy_digest(),
        PLANNING_CONTEXT,
    )
    .expect("a complete transition request");
    let detail = adapter
        .authorize_candidate_transition(&other)
        .expect_err("another validator must fail")
        .to_string();
    assert!(detail.contains("another validator"), "{detail}");

    // So is a request from another tuning session.
    let detail = adapter
        .authorize_candidate_transition(&transition_request(0x12, &first, &second))
        .expect_err("another session must fail")
        .to_string();
    assert!(detail.contains("bound tuning session"), "{detail}");
}

#[test]
fn a_run_receipt_carries_the_exact_run_intent() {
    let candidate = candidate(0.06, 0.35, 4.0);
    let digest = candidate_digest(&candidate);
    let document = mission_document("run-intent");
    let context = run_context(0x11, &document, digest, 17);
    let expected = context.digest().expect("the run intent digest");
    let mut adapter = adapter(0x11);

    let receipt = adapter
        .ensure_candidate_for_run_blocking(&capability(0x11), &context, &candidate, digest)
        .expect("activate the candidate for the run");
    assert_eq!(receipt.run_intent_digest, Some(expected));
    assert_eq!(receipt.requested_digest, digest);
    assert_eq!(receipt.applied_digest, digest);
    assert_eq!(receipt.readback_digest, digest);

    // A run intent that names another candidate is refused.
    let other = candidate_digest(&rig::candidate(0.06, 0.35, 4.4));
    let detail = adapter
        .ensure_candidate_for_run_blocking(&capability(0x11), &context, &candidate, other)
        .expect_err("another candidate must fail")
        .to_string();
    assert!(detail.contains("another candidate"), "{detail}");
}

#[test]
fn an_already_active_candidate_is_not_written_again() {
    let candidate = candidate(0.06, 0.35, 4.0);
    let digest = candidate_digest(&candidate);
    let (mut adapter, controller) = adapter_with_log(0x11);

    adapter
        .ensure_settled_candidate_blocking(&capability(0x11), &candidate, digest)
        .expect("settle the candidate");
    let after_first = controller.applies();
    assert_eq!(after_first, 1);

    adapter
        .ensure_settled_candidate_blocking(&capability(0x11), &candidate, digest)
        .expect("repeat the settled candidate");
    assert_eq!(
        controller.applies(),
        after_first,
        "reconciling an active candidate must write nothing"
    );
}

#[test]
fn the_supervised_process_request_receives_the_durable_run_intent_digest() {
    let candidate = candidate(0.06, 0.35, 4.0);
    let document = mission_document("supervised-launch");
    let context = run_context(0x11, &document, candidate_digest(&candidate), 23);
    let expected = context.digest().expect("the run intent digest");

    let request = bind_run_intent(rig::supervised_request(), expected);
    assert_eq!(request.run_intent_digest, expected);
    require_run_intent(&request, expected).expect("the launch carries the exact run intent");

    let other = run_context(0x11, &document, candidate_digest(&candidate), 24)
        .digest()
        .expect("another run intent digest");
    assert_ne!(other, expected);
    require_run_intent(&request, other)
        .expect_err("a launch for another run intent must fail closed");
    require_run_intent(&rig::supervised_request(), expected)
        .expect_err("a launch with no run intent must fail closed");
}

#[test]
fn a_production_input_change_changes_the_scenario_runtime_identity() {
    let first = runtime_identity("first");
    let second = runtime_identity("second");
    assert_ne!(first.identity(), second.identity());

    // The value the campaign keys a suite baseline on is the composed
    // scenario runtime identity, so the change has to reach that far.
    let first_runtime = scenario_runtime_identity(
        &aviate_action_port_identity(first.identity()).expect("the first action port"),
    )
    .expect("the first scenario runtime");
    let second_runtime = scenario_runtime_identity(
        &aviate_action_port_identity(second.identity()).expect("the second action port"),
    )
    .expect("the second scenario runtime");
    assert_ne!(first_runtime, second_runtime);
}

#[test]
fn the_factory_binds_the_runtime_identity_it_was_built_with() {
    let runtime = runtime_identity("factory");
    let factory = AviateVehicleFactory::new(
        rig::TestMapping::new(),
        rig::TestController(rig::ControllerHandle::new()),
        validator(),
        runtime.identity().clone(),
    )
    .expect("a complete factory");

    assert_eq!(factory.runtime_identity(), runtime.identity());
    assert_eq!(
        factory.transition_validator_identity(),
        validator().identity()
    );
    assert_eq!(
        factory.adjacency_policy_digest(),
        validator().policy_digest()
    );
    let expected_port =
        aviate_action_port_identity(runtime.identity()).expect("the action port identity");
    assert_eq!(factory.scenario_action_port_identity(), &expected_port);
    assert_eq!(
        factory.scenario_runtime_digest().expect("runtime digest"),
        scenario_runtime_identity(&expected_port)
            .expect("the scenario runtime")
            .digest
    );

    let binding = factory
        .bind_blocking(&capability(0x11))
        .expect("bind the vehicle to the validated session");
    drop(binding);
}

#[test]
fn a_restart_with_another_runtime_identity_fails_before_external_mutation() {
    let frozen = runtime_identity("frozen");
    let changed = runtime_identity("changed");
    let driver = AviateScenarioDriver::new(
        changed,
        vec![MissionCapability::KinematicTruth],
        tolerance(),
        NoDirectControl,
    )
    .expect("a driver for the changed runtime");

    driver
        .require_frozen_runtime(frozen.identity())
        .expect_err("a restart under another runtime identity must fail");
    assert!(driver.admitted_run().is_none(), "no run may be admitted");
    assert!(driver.seal().is_none(), "no run may be sealed");
}

#[test]
fn an_execution_retry_cannot_change_the_runtime_identity() {
    let runtime = runtime_identity("retry");
    let frozen = runtime.identity().clone();
    let mut driver = AviateScenarioDriver::new(
        runtime,
        vec![MissionCapability::KinematicTruth],
        tolerance(),
        NoDirectControl,
    )
    .expect("a driver");
    let candidate = candidate(0.06, 0.35, 4.0);
    let document = mission_document("retry");
    let context = run_context(0x11, &document, candidate_digest(&candidate), 31);

    for attempt in 0..3 {
        driver
            .prepare_blocking(&document, &context)
            .unwrap_or_else(|error| panic!("prepare attempt {attempt}: {error}"));
        driver
            .require_frozen_runtime(&frozen)
            .unwrap_or_else(|error| panic!("retry attempt {attempt}: {error}"));
        assert_eq!(
            driver.runtime_identity().identity(),
            &frozen,
            "a retry must keep one frozen runtime identity"
        );
        driver.cleanup_blocking().expect("clean up the attempt");
    }
}

#[test]
fn a_tampered_runtime_document_fails_its_own_attestation() {
    let runtime = runtime_identity("tamper");
    runtime.attest().expect("an untampered runtime attests");

    let mut document = runtime.document().clone();
    document.sources[0].sha256 = "0".repeat(64);
    let tampered = document.identity().expect("the tampered identity");
    assert_ne!(
        &tampered,
        runtime.identity(),
        "a tampered source entry must change the runtime identity"
    );

    // A document that no longer states a canonical inventory is refused
    // outright rather than given a second identity.
    let mut reordered = runtime.document().clone();
    reordered.sources.reverse();
    reordered
        .identity()
        .expect_err("a reordered inventory must be refused");

    let mut zeroed = runtime.document().clone();
    zeroed.adjacency_policy_digest = Digest::from_bytes([0; 32]);
    zeroed
        .identity()
        .expect_err("a zero policy digest must be refused");
}

#[test]
fn a_mission_the_run_intent_does_not_name_is_refused_before_start() {
    let runtime = runtime_identity("mission");
    let mut driver = AviateScenarioDriver::new(
        runtime,
        vec![MissionCapability::KinematicTruth],
        tolerance(),
        NoDirectControl,
    )
    .expect("a driver");
    let candidate = candidate(0.06, 0.35, 4.0);
    let admitted = mission_document("admitted");
    let other = mission_document("other");
    let context = run_context(0x11, &admitted, candidate_digest(&candidate), 41);

    let detail = driver
        .prepare_blocking(&other, &context)
        .expect_err("another mission must fail")
        .to_string();
    assert!(detail.contains("run intent names"), "{detail}");
    assert!(driver.admitted_run().is_none());
    driver
        .start_blocking()
        .expect_err("no run may start without an admitted mission");
}

fn tolerance() -> StartStateTolerance {
    StartStateTolerance {
        position_m: 0.5,
        heading_rad: 0.1,
        speed_mps: 0.2,
        dwell_ns: 500_000_000,
    }
}
