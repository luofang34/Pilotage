#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use flight_tune::{Digest, ExecutedSample, ExecutedUncertaintyReceipt};

use super::verify_executed_uncertainty;

#[path = "tests/support.rs"]
mod support;

use support::{RUN_SEED, sealed};

#[test]
fn a_run_the_core_sealed_is_derived_again_and_agrees() {
    let run = sealed(21);

    let verified = verify_executed_uncertainty(&run.receipt, &run.samples)
        .expect("the derived relation agrees");

    assert_eq!(verified.sample_count(), 21);
    assert_eq!(verified.receipt_digest(), run.receipt.receipt_digest);
    assert_eq!(
        verified.run_intent_digest(),
        run.receipt.launch.run_intent_digest
    );
}

#[test]
fn a_receipt_that_states_a_count_no_sample_produced_is_refused() {
    assert_refused(|receipt, _| {
        receipt.ledger.actuator.applied_hold = receipt.ledger.actuator.applied_hold.wrapping_add(1);
    });
}

#[test]
fn a_receipt_that_states_a_lane_change_no_sample_made_is_refused() {
    assert_refused(|receipt, _| {
        receipt.ledger.sensor_lanes[0].changed = 0;
    });
}

#[test]
fn a_receipt_that_hides_a_held_sample_is_refused() {
    assert_refused(|receipt, _| {
        receipt.ledger.sensor_lanes[0].held = 0;
    });
}

#[test]
fn a_sample_stream_that_lost_a_sample_is_refused() {
    assert_refused(|_, samples| {
        samples.pop();
    });
}

#[test]
fn a_sample_that_states_a_value_it_did_not_derive_is_refused() {
    assert_refused(|_, samples| {
        let mut sensor = samples[5].sensor.expect("sensor evidence");
        sensor.effective_value_bits[0] = sensor.raw_value_bits[0];
        samples[5].sensor = Some(sensor);
    });
}

#[test]
fn a_hold_decision_the_schedule_never_stated_is_refused() {
    assert_refused(|_, samples| {
        let mut actuator = samples[5].actuator.expect("actuator evidence");
        actuator.selected_hold = !actuator.selected_hold;
        actuator.applied_hold = actuator.selected_hold;
        samples[5].actuator = Some(actuator);
    });
}

#[test]
fn an_actuator_lane_that_carries_another_scale_is_refused() {
    assert_refused(|_, samples| {
        let mut actuator = samples[5].actuator.expect("actuator evidence");
        actuator.authority_scaled_lane_bits[0] = actuator.requested_lane_bits[0];
        samples[5].actuator = Some(actuator);
    });
}

#[test]
fn a_launch_that_names_another_seed_than_the_declaration_is_refused() {
    assert_refused(|receipt, _| {
        receipt.launch.run_seed = RUN_SEED.wrapping_add(1);
    });
}

#[test]
fn a_declaration_whose_capabilities_do_not_follow_from_its_factors_is_refused() {
    assert_refused(|receipt, _| {
        receipt.declaration.required_capabilities.pop();
        receipt.launch.required_capabilities.pop();
    });
}

#[test]
fn an_active_online_hover_estimator_is_refused() {
    assert_refused(|_, samples| {
        for sample in samples.iter_mut() {
            sample.hover.estimator_disabled = false;
        }
    });
}

#[test]
fn a_receipt_whose_identity_does_not_cover_its_content_is_refused() {
    let run = sealed(21);
    let mut receipt = run.receipt.clone();
    receipt.receipt_digest = Digest::from_bytes([3; 32]);

    assert!(verify_executed_uncertainty(&receipt, &run.samples).is_err());
}

/// Changes one verified run and requires the derived relation to refuse it.
///
/// The receipt is sealed again over the changed content, so a refusal is the
/// relation failing rather than an identity that no longer covers its own
/// document.
fn assert_refused(change: impl FnOnce(&mut ExecutedUncertaintyReceipt, &mut Vec<ExecutedSample>)) {
    let run = sealed(21);
    assert!(
        verify_executed_uncertainty(&run.receipt, &run.samples).is_ok(),
        "the unchanged run must verify"
    );

    let mut receipt = run.receipt;
    let mut samples = run.samples;
    change(&mut receipt, &mut samples);
    receipt.sample_stream_digest = super::stream_digest(&samples).expect("stream identity");
    receipt.receipt_digest = super::receipt_digest(&receipt).expect("receipt identity");

    assert!(
        verify_executed_uncertainty(&receipt, &samples).is_err(),
        "the changed run must be refused"
    );
}
