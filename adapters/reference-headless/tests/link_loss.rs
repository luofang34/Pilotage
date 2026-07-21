//! Link-loss policy behavior through the public `VehicleAdapter` API
//! (ADR-0008).
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{LinkLossPolicy, StepBudget, VehicleAdapter};
use pilotage_adapter_reference::ReferenceAdapter;
use pilotage_protocol::{
    ControlPayload, Generation, LogicalAxisId, ScopeId, ScopedControlFrame, SequenceNum, SessionId,
    VehicleId,
};
use pilotage_timing::MonoTimestamp;

fn full_throttle_frame(vehicle: VehicleId) -> ScopedControlFrame {
    ScopedControlFrame {
        session: SessionId::new(1),
        vehicle,
        scope: ScopeId::new("vehicle.motion"),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        payload: ControlPayload {
            axes: vec![(LogicalAxisId::new(2), 1.0), (LogicalAxisId::new(3), 0.0)],
            edges: vec![],
        },
    }
}

fn motion_scope() -> ScopeId {
    ScopeId::new("vehicle.motion")
}

fn measured_speed(adapter: &mut ReferenceAdapter) -> f64 {
    adapter.sample_telemetry().samples[0]
        .speed
        .expect("reference adapter speed")
}

#[test]
fn neutralize_zeroes_controls_and_speed_decays_via_drag() {
    let vehicle = VehicleId::new(1);
    let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
    adapter.apply_control(&full_throttle_frame(vehicle));

    // Build up speed under full throttle before losing the link.
    for _ in 0..50u32 {
        adapter.step(StepBudget { ticks: 1 });
    }
    let speed_before_loss = measured_speed(&mut adapter);
    assert!(speed_before_loss > 0.0);

    adapter
        .set_link_loss_policy(vehicle, &motion_scope(), Some(LinkLossPolicy::Neutralize))
        .expect("policy enacted");
    adapter.step(StepBudget { ticks: 1 });
    let speed_after_one_tick = measured_speed(&mut adapter);
    assert!(speed_after_one_tick < speed_before_loss);

    // With controls neutralized, speed keeps decaying toward zero under drag
    // alone rather than being sustained by throttle.
    for _ in 0..500u32 {
        adapter.step(StepBudget { ticks: 1 });
    }
    let speed_far_after = measured_speed(&mut adapter);
    assert!(speed_far_after < speed_after_one_tick);
    assert!(speed_far_after >= 0.0);
}

#[test]
fn clearing_link_loss_policy_restores_normal_control() {
    let vehicle = VehicleId::new(4);
    let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
    adapter.apply_control(&full_throttle_frame(vehicle));

    adapter
        .set_link_loss_policy(vehicle, &motion_scope(), Some(LinkLossPolicy::Neutralize))
        .expect("policy enacted");
    adapter.step(StepBudget { ticks: 1 });
    let speed_while_neutralized = measured_speed(&mut adapter);

    // Link recovery: clearing the policy and re-applying full throttle must
    // resume acceleration rather than staying neutralized permanently.
    adapter
        .set_link_loss_policy(vehicle, &motion_scope(), None)
        .expect("policy enacted");
    adapter.apply_control(&full_throttle_frame(vehicle));
    for _ in 0..50u32 {
        adapter.step(StepBudget { ticks: 1 });
    }
    let speed_after_recovery = measured_speed(&mut adapter);
    assert!(speed_after_recovery > speed_while_neutralized);
}

#[test]
fn hold_brief_holds_controls_then_neutralizes() {
    let vehicle = VehicleId::new(2);
    let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
    adapter.apply_control(&full_throttle_frame(vehicle));

    adapter
        .set_link_loss_policy(
            vehicle,
            &motion_scope(),
            Some(LinkLossPolicy::HoldBrief { ticks: 3 }),
        )
        .expect("policy enacted");

    // While held, throttle keeps applying: speed should still be climbing
    // for the held ticks (comparable to no link loss at all).
    let mut without_loss = ReferenceAdapter::from_seed(vehicle, 1);
    without_loss.apply_control(&full_throttle_frame(vehicle));

    for _ in 0..3u32 {
        adapter.step(StepBudget { ticks: 1 });
        without_loss.step(StepBudget { ticks: 1 });
    }
    let held_speed = measured_speed(&mut adapter);
    let unaffected_speed = measured_speed(&mut without_loss);
    assert_eq!(held_speed, unaffected_speed);

    // After the hold window elapses, controls neutralize: further stepping
    // must decay speed rather than keep climbing under throttle.
    adapter.step(StepBudget { ticks: 1 });
    let speed_after_neutralize_tick = measured_speed(&mut adapter);

    without_loss.step(StepBudget { ticks: 1 });
    let unaffected_speed_next = measured_speed(&mut without_loss);

    assert!(speed_after_neutralize_tick < unaffected_speed_next);
}

#[test]
fn hold_brief_expiry_is_not_undone_by_a_later_apply_control() {
    let vehicle = VehicleId::new(3);
    let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
    adapter.apply_control(&full_throttle_frame(vehicle));
    adapter
        .set_link_loss_policy(
            vehicle,
            &motion_scope(),
            Some(LinkLossPolicy::HoldBrief { ticks: 1 }),
        )
        .expect("policy enacted");

    // Run past the hold window so the policy has neutralized.
    adapter.step(StepBudget { ticks: 2 });
    let speed_after_neutralize = measured_speed(&mut adapter);

    // A new control frame arrives without the link ever being recovered via
    // `set_link_loss_policy(vehicle, None)`. Per the `VehicleAdapter` trait's
    // documented invariant, `None` is the only path back to normal control,
    // so this frame must not un-neutralize the vehicle.
    adapter.apply_control(&full_throttle_frame(vehicle));
    adapter.step(StepBudget { ticks: 50 });
    let speed_after_new_frame = measured_speed(&mut adapter);

    assert!(speed_after_new_frame <= speed_after_neutralize);
}
